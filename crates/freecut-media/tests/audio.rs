//! Sample-accurate audio reads and waveform peaks.
#![cfg(feature = "ffmpeg")]
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod support;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use freecut_media::{AudioReader, CancelToken, DecodeError, StreamKind, audio_peaks};
use freecut_timeline::time::{SampleRate, TICKS_PER_SECOND, Tick};
use support::{AudioSpec, Rng, VideoSpec};

const SECOND: i64 = TICKS_PER_SECOND;

fn wav_48k() -> (PathBuf, Vec<i16>) {
    let frames = 48_000 * 3 + 123;
    let path = support::cached("tone_48k.wav", |p| {
        support::write_wav(p, 48_000, 2, frames);
    });
    (path, support::wav_samples(48_000, 2, frames))
}

fn wav_44k() -> PathBuf {
    support::cached("tone_44k.wav", |p| {
        support::write_wav(p, 44_100, 2, 44_100 * 3);
    })
}

fn aac_48k() -> PathBuf {
    support::cached("tone_48k.m4a", |p| {
        support::encode_aac(
            p,
            &AudioSpec {
                rate: 48_000,
                channels: 2,
                samples: 48_000 * 4,
            },
        );
    })
}

fn av_clip() -> PathBuf {
    support::cached("av_25.mp4", |p| {
        let mut spec = VideoSpec::barcode(75, (25, 1));
        spec.audio = Some(AudioSpec {
            rate: 48_000,
            channels: 2,
            samples: 48_000 * 3,
        });
        support::encode_video(p, &spec);
    })
}

fn rate(hz: u32) -> SampleRate {
    SampleRate::new(hz).unwrap()
}

/// Output-sample start time at `rate`.
fn at(n: i64, hz: u32) -> Tick {
    Tick::from_raw(n * (TICKS_PER_SECOND / i64::from(hz)))
}

/// Reads the whole stream sequentially in uneven chunks.
fn sequential(path: &Path, stream: usize, hz: u32, ch: u16, total: usize) -> Vec<f32> {
    let mut r = AudioReader::open(path, stream, rate(hz), ch).unwrap();
    let mut out = Vec::new();
    let mut pos = 0usize;
    let mut k = 0usize;
    while pos < total {
        let n = [1000, 1, 4096, 333, 2048][k % 5].min(total - pos);
        out.extend(r.read(at(pos as i64, hz), n).unwrap());
        pos += n;
        k += 1;
    }
    out
}

/// Random spans (in random order) compared with the sequential read.
fn random_matches_sequential(path: &Path, stream: usize, hz: u32, ch: u16, total: usize, eps: f32) {
    let seq = sequential(path, stream, hz, ch, total);
    let mut r = AudioReader::open(path, stream, rate(hz), ch).unwrap();
    let mut rng = Rng::new(u64::from(hz) ^ u64::from(ch));
    let c = usize::from(ch);
    for _ in 0..40 {
        let start = rng.range(0, total as i64 - 10) as usize;
        let len = (rng.range(1, 6000) as usize).min(total - start);
        let got = r.read(at(start as i64, hz), len).unwrap();
        let want = &seq[start * c..(start + len) * c];
        let worst = got
            .iter()
            .zip(want)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(
            worst <= eps,
            "read at {start} (+{len}) differs from sequential by {worst}"
        );
    }
}

#[test]
fn pcm_at_the_native_rate_is_bit_exact() {
    let (path, samples) = wav_48k();
    let frames = samples.len() / 2;
    let seq = sequential(&path, 0, 48_000, 2, frames + 500);
    for (i, s) in samples.iter().enumerate() {
        assert_eq!(
            seq[i].to_bits(),
            (f32::from(*s) / 32_768.0).to_bits(),
            "sample {i}"
        );
    }
    // Zeros after the end.
    assert!(seq[samples.len()..].iter().all(|v| *v == 0.0));
    random_matches_sequential(&path, 0, 48_000, 2, frames, 0.0);
}

#[test]
fn reads_before_the_start_are_silent() {
    let (path, samples) = wav_48k();
    let mut r = AudioReader::open(&path, 0, rate(48_000), 2).unwrap();
    let out = r.read(at(-100, 48_000), 150).unwrap();
    assert!(out[..200].iter().all(|v| *v == 0.0));
    assert_eq!(out[200], f32::from(samples[0]) / 32_768.0);
    // A start between samples floors to the sample covering it.
    let out = r.read(Tick::from_raw(14_700 * 10 + 5), 1).unwrap();
    assert_eq!(out[0], f32::from(samples[20]) / 32_768.0);
}

#[test]
fn resampled_reads_match_sequential() {
    random_matches_sequential(&wav_44k(), 0, 48_000, 2, 48_000 * 3, 1e-6);
}

#[test]
fn reads_across_the_end_match_sequential_and_go_silent() {
    let path = wav_44k();
    let total = 48_000 * 3 + 2000;
    let seq = sequential(&path, 0, 48_000, 2, total);
    // 3 s of 44.1 kHz input ends exactly at output sample 144000.
    let end = 48_000 * 3;
    assert!(seq[(end - 100) * 2..end * 2].iter().any(|v| *v != 0.0));
    assert!(seq[end * 2..].iter().all(|v| *v == 0.0));
    for start in [end - 1500, end - 1, end, end + 10] {
        let mut r = AudioReader::open(&path, 0, rate(48_000), 2).unwrap();
        let got = r.read(at(start as i64, 48_000), 1800).unwrap();
        let want = &seq[start * 2..(start + 1800) * 2];
        let worst = got
            .iter()
            .zip(want)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(worst <= 1e-6, "read at {start} differs by {worst}");
    }
}

#[test]
fn downmixed_reads_match_sequential() {
    random_matches_sequential(&wav_44k(), 0, 44_100, 1, 44_100 * 3, 1e-6);
}

#[test]
fn aac_random_reads_match_sequential() {
    random_matches_sequential(&aac_48k(), 0, 48_000, 2, 48_000 * 4, 1e-6);
    random_matches_sequential(&aac_48k(), 0, 22_050, 1, 22_050 * 4, 1e-6);
}

#[test]
fn audio_in_a_video_file_is_aligned_to_the_picture() {
    let path = av_clip();
    let probed = freecut_media::probe(&path).unwrap();
    let a = &probed.info.audio[0];
    let index = a.index as usize;
    let total = 48_000 * 3;
    random_matches_sequential(&path, index, 48_000, 2, total, 1e-6);
    let mut r = AudioReader::open(&path, index, rate(48_000), 2).unwrap();
    assert_eq!(r.native_rate(), 48_000);
    assert_eq!(r.native_channels(), 2);
    // Silence before the stream's start, sound after it.
    let start = a.start;
    let before = r
        .read(Tick::from_raw(start.raw() - SECOND / 10), 4800)
        .unwrap();
    assert!(before.iter().all(|v| *v == 0.0));
    let after = r
        .read(start.checked_add(Tick::from_raw(SECOND / 2)).unwrap(), 4800)
        .unwrap();
    assert!(after.iter().any(|v| v.abs() > 0.05));
}

#[test]
fn opening_bad_streams_and_arguments() {
    let (path, _) = wav_48k();
    assert!(matches!(
        AudioReader::open(&path, 0, rate(48_000), 0),
        Err(DecodeError::InvalidArgument(_))
    ));
    assert!(matches!(
        AudioReader::open(&av_clip(), 0, rate(48_000), 2),
        Err(DecodeError::NoSuchStream {
            index: 0,
            kind: StreamKind::Audio
        })
    ));
}

/// Brute-force peaks from a native-rate sequential read.
fn brute_force(path: &Path, stream: usize, bucket: Tick) -> Vec<(f32, f32)> {
    let mut r = AudioReader::open(path, stream, rate(48_000), 2).unwrap();
    assert_eq!(r.native_rate(), 48_000);
    let total = 48_000 * 5;
    // Past the end the reader returns zeros; callers compare a prefix.
    let all = r.read(Tick::ZERO, total).unwrap();
    let per_bucket = (bucket.raw() / (TICKS_PER_SECOND / 48_000)) as usize;
    let mut out = Vec::new();
    for chunk in all.chunks(per_bucket * 2) {
        let lo = chunk.iter().copied().fold(f32::INFINITY, f32::min);
        let hi = chunk.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        out.push((lo, hi));
    }
    out
}

#[test]
fn peaks_equal_a_brute_force_min_max() {
    let (path, samples) = wav_48k();
    let bucket = Tick::from_raw(SECOND / 100); // 480 samples
    let peaks = audio_peaks(&path, 0, bucket, &CancelToken::new()).unwrap();
    let frames = samples.len() / 2;
    assert_eq!(peaks.len(), frames.div_ceil(480));
    let brute = brute_force(&path, 0, bucket);
    assert_eq!(&peaks[..], &brute[..peaks.len()]);
    // The final, partial bucket holds the last samples.
    let tail = &samples[(peaks.len() - 1) * 480 * 2..];
    let lo = tail
        .iter()
        .map(|s| f32::from(*s) / 32_768.0)
        .fold(f32::INFINITY, f32::min);
    assert_eq!(peaks.last().unwrap().0, lo);

    let aac = aac_48k();
    let peaks = audio_peaks(&aac, 0, bucket, &CancelToken::new()).unwrap();
    let brute = brute_force(&aac, 0, bucket);
    assert_eq!(peaks.len(), 400);
    assert_eq!(&peaks[..], &brute[..peaks.len()]);
}

#[test]
fn peaks_validate_and_honour_cancellation() {
    let (path, _) = wav_48k();
    assert!(matches!(
        audio_peaks(&path, 0, Tick::ZERO, &CancelToken::new()),
        Err(DecodeError::InvalidArgument(_))
    ));
    let cancelled = CancelToken::new();
    cancelled.cancel();
    assert!(matches!(
        audio_peaks(&path, 0, Tick::from_raw(SECOND), &cancelled),
        Err(DecodeError::Cancelled)
    ));
    let expired = CancelToken::new().with_deadline(Instant::now());
    assert!(matches!(
        audio_peaks(&path, 0, Tick::from_raw(SECOND), &expired),
        Err(DecodeError::Timeout)
    ));
}

#[test]
fn cancel_stops_peaks_quickly() {
    // 40 minutes of 16 kHz mono: far more work than the cancel delay.
    let path = support::cached("long_16k.wav", |p| {
        let frames = 16_000 * 60 * 40;
        let mut bytes = Vec::with_capacity(44 + frames * 2);
        let data_len = (frames * 2) as u32;
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&16_000u32.to_le_bytes());
        bytes.extend_from_slice(&32_000u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..frames {
            bytes.extend_from_slice(&(((i % 200) as i16 - 100) * 100).to_le_bytes());
        }
        std::fs::write(p, bytes).unwrap();
    });
    let token = CancelToken::new();
    let remote = token.clone();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        remote.cancel();
        Instant::now()
    });
    let result = audio_peaks(&path, 0, Tick::from_raw(SECOND / 100), &token);
    let returned = Instant::now();
    let cancelled_at = canceller.join().unwrap();
    assert!(matches!(result, Err(DecodeError::Cancelled)), "{result:?}");
    let lag = returned.saturating_duration_since(cancelled_at);
    assert!(lag < Duration::from_millis(250), "took {lag:?} to stop");
}

#[test]
fn audio_starting_after_the_picture_keeps_its_offset() {
    require_ffmpeg_cli!();
    let src = av_clip();
    let path = support::dir().join("late_audio.mp4");
    let s = src.to_str().unwrap();
    support::run_ffmpeg(&[
        "-i",
        s,
        "-itsoffset",
        "0.5",
        "-i",
        s,
        "-map",
        "0:v",
        "-map",
        "1:a",
        "-c",
        "copy",
        path.to_str().unwrap(),
    ]);
    let info = freecut_media::probe(&path).unwrap().info;
    assert_eq!(info.video[0].start, Tick::ZERO);
    let a = &info.audio[0];
    // A stream-copy remux re-times the AAC priming frame (1024 samples) and
    // rounds the edit list to milliseconds, so the stream starts up to one
    // frame early.
    let half = Tick::from_raw(SECOND / 2);
    let slack = (1024 + 48) * (SECOND / 48_000);
    assert!(
        a.start <= half && a.start.abs_diff(half) <= slack as u64,
        "audio starts at {:?}",
        a.start
    );
    let mut r = AudioReader::open(&path, a.index as usize, rate(48_000), 2).unwrap();
    let first_sample = a.start.raw() / (SECOND / 48_000);
    let before = r.read(Tick::ZERO, first_sample as usize).unwrap();
    assert!(
        before.iter().all(|v| *v == 0.0),
        "silence before the audio starts"
    );
    let after = r
        .read(
            a.start.checked_add(Tick::from_raw(SECOND / 10)).unwrap(),
            4800,
        )
        .unwrap();
    assert!(after.iter().any(|v| v.abs() > 0.05));
    random_matches_sequential(&path, a.index as usize, 48_000, 2, 48_000 * 3, 1e-6);
}

/// A 1 s 48 kHz `pcm_f32le` WAV of `aevalsrc` expression `exprs` (one per
/// channel, `|`-separated) with channel layout `layout`, made by the CLI.
fn constant_wav(name: &str, exprs: &str, layout: &str) -> PathBuf {
    support::cached(name, |p| {
        let src = format!("aevalsrc={exprs}:c={layout}:s=48000:d=1");
        support::run_ffmpeg(&[
            "-f",
            "lavfi",
            "-i",
            &src,
            "-c:a",
            "pcm_f32le",
            p.to_str().unwrap(),
        ]);
    })
}

/// The stereo frame in the middle of the file, read at 48 kHz and at
/// 44.1 kHz (through the resampler both ways).
fn middle_frames(path: &Path) -> [[f32; 2]; 2] {
    [48_000u32, 44_100].map(|hz| {
        let mut r = AudioReader::open(path, 0, rate(hz), 2).unwrap();
        let mid = i64::from(hz) / 2;
        let v = r.read(at(mid, hz), 1).unwrap();
        [v[0], v[1]]
    })
}

fn near(got: [f32; 2], want: [f32; 2], eps: f32) -> bool {
    (got[0] - want[0]).abs() <= eps && (got[1] - want[1]).abs() <= eps
}

#[test]
fn mono_plays_on_both_channels_at_unity() {
    require_ffmpeg_cli!();
    let path = constant_wav("dc_mono.wav", "0.5", "mono");
    let [native, resampled] = middle_frames(&path);
    // AU-010: duplicated at unity, not lowered by 3 dB (0.3536).
    assert_eq!(native, [0.5, 0.5]);
    assert!(near(resampled, [0.5, 0.5], 1e-4), "{resampled:?}");
}

#[test]
fn stereo_passes_through_unchanged() {
    require_ffmpeg_cli!();
    let path = constant_wav("dc_stereo.wav", "0.5|-0.25", "stereo");
    let [native, resampled] = middle_frames(&path);
    assert_eq!(native, [0.5, -0.25]);
    assert!(near(resampled, [0.5, -0.25], 1e-4), "{resampled:?}");
}

#[test]
fn five_one_downmixes_with_itu_coefficients() {
    require_ffmpeg_cli!();
    // FL FR FC LFE SL SR.
    let path = constant_wav("dc_51.wav", "0.1|0.2|0.3|0.9|0.05|0.07", "5.1(side)");
    let g = std::f32::consts::FRAC_1_SQRT_2;
    // Centre and surrounds at −3 dB, LFE dropped, no normalisation.
    let want = [0.1 + g * (0.3 + 0.05), 0.2 + g * (0.3 + 0.07)];
    let [native, resampled] = middle_frames(&path);
    assert!(near(native, want, 1e-6), "{native:?} vs {want:?}");
    assert!(near(resampled, want, 1e-4), "{resampled:?} vs {want:?}");
}
