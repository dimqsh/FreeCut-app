//! Regression tests from the M2 media review (M2-MEDIA-01 … 11). Each test
//! reproduces one reported defect on a fixture made at test time.
#![cfg(feature = "ffmpeg")]
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod support;

use std::path::{Path, PathBuf};

use freecut_media::color::HdrTransfer;
use freecut_media::{
    AudioReader, CancelToken, ProbeError, SeekMode, VideoDecoder, audio_peaks, probe,
};
use freecut_timeline::time::{Rational, SampleRate, TICKS_PER_SECOND, Tick};
use support::{AudioSpec, VideoSpec};

const SECOND: i64 = TICKS_PER_SECOND;

/// Runs the `ffmpeg` CLI with `args` and the output file `name` (in the
/// fixture directory) appended; returns the output path.
fn cli(name: &str, args: &[&str]) -> PathBuf {
    let path = support::dir().join(name);
    let mut all: Vec<&str> = args.to_vec();
    all.push(path.to_str().unwrap());
    support::run_ffmpeg(&all);
    path
}

fn write(name: &str, bytes: &[u8]) -> PathBuf {
    let p = support::dir().join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

/// Reads `total` frames of a stream sequentially in 4096-frame reads.
fn seq_read(path: &Path, stream: usize, hz: u32, ch: u16, total: usize) -> Vec<f32> {
    let mut r = AudioReader::open(path, stream, SampleRate::new(hz).unwrap(), ch).unwrap();
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < total {
        let n = 4096.min(total - pos);
        let t = Tick::from_raw(pos as i64 * (SECOND / i64::from(hz)));
        out.extend(r.read(t, n).unwrap());
        pos += n;
    }
    out
}

/// Random 4800-frame reads compared with a sequential read: no sample may
/// differ by more than `eps`.
fn assert_random_reads_match(path: &Path, hz: u32, ch: u16, total: usize, seed: u64, eps: f32) {
    let seq = seq_read(path, 0, hz, ch, total);
    let at = |n: i64| Tick::from_raw(n * (SECOND / i64::from(hz)));
    let mut r = AudioReader::open(path, 0, SampleRate::new(hz).unwrap(), ch).unwrap();
    let mut rng = support::Rng::new(seed);
    let c = usize::from(ch);
    for _ in 0..30 {
        let start = rng.range(i64::from(hz), total as i64 - 10_000) as usize;
        let got = r.read(at(start as i64), 4800).unwrap();
        let worst = got
            .iter()
            .zip(&seq[start * c..(start + 4800) * c])
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(
            worst <= eps,
            "{}: random read at {start} differs by {worst}",
            path.display()
        );
    }
}

/// M2-MEDIA-01: a stream-copy trim of a long-GOP file needs more than 512
/// packets before the first presentable video frame; audio must still open.
#[test]
fn f2_audio_of_a_trimmed_long_gop_mp4_opens() {
    require_ffmpeg_cli!();
    let src = cli(
        "f2_src.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=192x144:rate=60",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "20",
            "-c:v",
            "libx264",
            "-bf",
            "3",
            "-g",
            "600",
            "-keyint_min",
            "600",
            "-sc_threshold",
            "0",
            "-c:a",
            "aac",
            "-pix_fmt",
            "yuv420p",
        ],
    );
    let cut = cli(
        "f2_cut.mp4",
        &["-ss", "9.5", "-i", src.to_str().unwrap(), "-c", "copy"],
    );
    let info = probe(&cut).unwrap().info;
    let a = info.audio[0].index as usize;
    let v = info.video[0].index as usize;
    VideoDecoder::open(&cut, v).unwrap();
    let r = AudioReader::open(&cut, a, SampleRate::new(48_000).unwrap(), 2);
    assert!(
        r.is_ok(),
        "probe succeeded but AudioReader::open failed: {:?}",
        r.err()
    );
    assert!(audio_peaks(&cut, a, Tick::from_raw(SECOND / 10), &CancelToken::new()).is_ok());
    // The reader agrees with the probe about where the audio starts.
    let mut r = r.unwrap();
    let start = info.audio[0].start;
    let first_sample = start.raw().div_euclid(SECOND / 48_000);
    if first_sample > 0 {
        let before = r.read(Tick::ZERO, first_sample as usize).unwrap();
        assert!(before.iter().all(|s| *s == 0.0));
    }
    let after = r
        .read(start.checked_add(Tick::from_raw(SECOND / 2)).unwrap(), 4800)
        .unwrap();
    assert!(after.iter().any(|s| s.abs() > 0.05));
}

/// M2-MEDIA-02: Matroska's 1 ms timestamps must not shift random reads
/// against a sequential read.
#[test]
fn i1_aac_in_matroska_random_reads_match_sequential() {
    let p = support::cached("i1_tone.mkv", |p| {
        support::encode_aac(
            p,
            &AudioSpec {
                rate: 48_000,
                channels: 2,
                samples: 48_000 * 4,
            },
        );
    });
    assert_random_reads_match(&p, 48_000, 2, 48_000 * 4, 77, 1e-6);
}

/// M2-MEDIA-02 with other codecs in Matroska: Opus (`.mka`), MP3 and AC-3
/// (fixed frame lengths, snapped onto the counted lattice) and Vorbis (frames
/// of varying length, read forward only). AC-3 fills unallocated mantissas
/// with noise from a generator whose state a seek can't restore (in any
/// container), so it is compared at that noise level (~2e-5); a misplaced
/// frame would differ by about 0.1.
#[test]
fn i1b_other_codecs_in_matroska_random_reads_match_sequential() {
    require_ffmpeg_cli!();
    let cases: [(&str, &[&str], f32); 4] = [
        ("i1b_opus.mka", &["-c:a", "libopus"], 1e-6),
        ("i1b_mp3.mkv", &["-c:a", "libmp3lame"], 1e-6),
        ("i1b_ac3.mkv", &["-c:a", "ac3"], 1e-4),
        (
            "i1b_vorbis.webm",
            &["-c:a", "vorbis", "-strict", "-2"],
            1e-6,
        ),
    ];
    for (k, (name, codec, eps)) in cases.into_iter().enumerate() {
        let mut args = vec![
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=6",
            "-ac",
            "2",
        ];
        args.extend_from_slice(codec);
        let p = cli(name, &args);
        assert_random_reads_match(&p, 48_000, 2, 48_000 * 6, 78 + k as u64, eps);
    }
}

/// M2-MEDIA-02, frames whose length changes mid-stream (PCM in Matroska,
/// 1000- then 1500-sample blocks): once the reader sees the lattice break it
/// reads forward only, and reads that relied on a seek are redone, so random
/// reads still equal the sequential read bit for bit.
#[test]
fn i1c_matroska_frames_of_changing_length_read_exactly() {
    require_ffmpeg_cli!();
    let p = cli(
        "i1c_mixed_blocks.mkv",
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=3:samples_per_frame=1000",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=660:sample_rate=48000:duration=3:samples_per_frame=1500",
            "-filter_complex",
            "[0:a][1:a]concat=n=2:v=0:a=1",
            "-c:a",
            "pcm_s16le",
            "-ac",
            "2",
        ],
    );
    assert_random_reads_match(&p, 48_000, 2, 48_000 * 6, 91, 0.0);
}

/// M2-MEDIA-03: Matroska stream durations come from each track's own
/// `DURATION` tag, not the container's end.
#[test]
fn n1_matroska_per_stream_durations() {
    require_ffmpeg_cli!();
    let p = cli(
        "n1_short_video.mkv",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=192x144:rate=25:duration=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=4",
            "-c:v",
            "libx264",
            "-c:a",
            "aac",
        ],
    );
    let info = probe(&p).unwrap().info;
    let v = &info.video[0];
    assert!(
        v.duration.abs_diff(Tick::from_raw(2 * SECOND)) <= (SECOND / 25) as u64,
        "2 s video stream reported as {:.3} s",
        v.duration.raw() as f64 / SECOND as f64
    );
    let a = &info.audio[0];
    assert!(
        a.duration.abs_diff(Tick::from_raw(4 * SECOND)) <= (SECOND / 20) as u64,
        "4 s audio stream reported as {:.3} s",
        a.duration.raw() as f64 / SECOND as f64
    );
    assert_eq!(info.duration, a.start.checked_add(a.duration).unwrap());
}

/// M2-MEDIA-04: VP9 with side-channel alpha in WebM keeps its alpha.
#[test]
fn l1_vp9_alpha_webm() {
    require_ffmpeg_cli!();
    let p = cli(
        "l1_alpha.webm",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=64x48:rate=25,format=rgba,geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='if(lt(X,32),255,0)'",
            "-t",
            "1",
            "-c:v",
            "libvpx-vp9",
            "-pix_fmt",
            "yuva420p",
            "-auto-alt-ref",
            "0",
        ],
    );
    let info = probe(&p).unwrap().info;
    assert!(
        info.video[0].has_alpha,
        "VP9 alpha WebM probed without alpha"
    );
    let mut d = VideoDecoder::open(&p, 0).unwrap();
    for t in [Tick::ZERO, Tick::from_raw(SECOND / 2)] {
        let f = d.frame_at(t, SeekMode::Exact).unwrap();
        assert_eq!(
            f.rgba[(10 * 64 + 50) * 4 + 3],
            0,
            "transparent pixel decoded opaque"
        );
        assert_eq!(
            f.rgba[(10 * 64 + 10) * 4 + 3],
            255,
            "opaque pixel decoded transparent"
        );
    }
}

/// M2-MEDIA-05: CFR 29.97 in Matroska's 1 ms time base is 30000/1001 and
/// not VFR.
#[test]
fn f1_matroska_2997_cfr_is_not_vfr() {
    let p = support::cached("f1_2997.mkv", |p| {
        support::encode_video(p, &VideoSpec::barcode(90, (30_000, 1001)));
    });
    let v = probe(&p).unwrap().info.video[0].clone();
    assert_eq!(
        v.rate,
        Some(Rational::new(30_000, 1001).unwrap()),
        "nominal rate"
    );
    assert!(!v.variable_rate, "CFR 29.97 with 1 ms rounding flagged VFR");
}

/// M2-MEDIA-06: a transport stream whose first IDR is more than 5 s in
/// keeps its video stream.
#[test]
fn g2b_ts_with_8s_gop_cut_mid_gop_keeps_video() {
    require_ffmpeg_cli!();
    let src = cli(
        "g2b_src.ts",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=640x360:rate=30",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "30",
            "-c:v",
            "libx264",
            "-bf",
            "3",
            "-g",
            "240",
            "-keyint_min",
            "240",
            "-sc_threshold",
            "0",
            "-c:a",
            "aac",
        ],
    );
    let data = std::fs::read(&src).unwrap();
    let cut = write("g2b_cut.ts", &data[(data.len() / 100 / 188) * 188..]);
    let info = probe(&cut).unwrap().info;
    assert_eq!(
        info.video.len(),
        1,
        "video stream silently dropped; probe says {info:?}"
    );
    assert_eq!((info.video[0].width, info.video[0].height), (640, 360));
    assert_eq!(info.audio.len(), 1);
}

/// M2-MEDIA-07: an animated GIF is a still of its first frame (PM-034), so
/// the decoder shows that frame at every time.
#[test]
fn g4_animated_gif_is_a_still_of_its_first_frame() {
    require_ffmpeg_cli!();
    let gif = cli(
        "g4.gif",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=64x48:rate=10",
            "-frames:v",
            "10",
        ],
    );
    assert!(probe(&gif).unwrap().info.still.is_some());
    let mut d = VideoDecoder::open(&gif, 0).unwrap();
    let a = d.frame_at(Tick::ZERO, SeekMode::Exact).unwrap();
    let b = d
        .frame_at(Tick::from_raw(SECOND / 2), SeekMode::Exact)
        .unwrap();
    assert_eq!(
        a.rgba, b.rgba,
        "a 'still' shows different pictures at 0 and 0.5 s"
    );
    let c = d
        .frame_at(Tick::from_raw(5 * SECOND), SeekMode::Exact)
        .unwrap();
    assert_eq!(a.rgba, c.rgba);
    // The poster shows the same first frame.
    let poster = freecut_media::poster(&gif, 64).unwrap();
    assert_eq!((poster.width, poster.height), (64, 48));
}

/// M2-MEDIA-08: a recognised MP4 with a damaged header is `Corrupt`.
#[test]
fn f3_damaged_mp4_header_is_corrupt_not_not_media() {
    require_ffmpeg_cli!();
    let src = cli(
        "f3_src.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=192x144:rate=25",
            "-t",
            "2",
            "-c:v",
            "libx264",
            "-movflags",
            "+faststart",
        ],
    );
    let mut bytes = std::fs::read(&src).unwrap();
    let j = bytes.windows(4).position(|w| w == b"stsd").unwrap();
    for b in &mut bytes[j + 4..j + 60] {
        *b = 0xff;
    }
    let err = probe(&write("f3_bad_stsd.mp4", &bytes)).unwrap_err();
    assert!(matches!(err, ProbeError::Corrupt { .. }), "{err:?}");
}

/// M2-MEDIA-08: an MP4 truncated inside its `moov` box is `Corrupt` too
/// (it has an index, just a damaged one).
#[test]
fn f3b_mp4_truncated_inside_moov_is_corrupt() {
    require_ffmpeg_cli!();
    let src = cli(
        "f3b_src.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=192x144:rate=25",
            "-t",
            "2",
            "-c:v",
            "libx264",
            "-movflags",
            "+faststart",
        ],
    );
    let bytes = std::fs::read(&src).unwrap();
    let j = bytes.windows(4).position(|w| w == b"stsd").unwrap();
    let err = probe(&write("f3b_cut_moov.mp4", &bytes[..j + 20])).unwrap_err();
    assert!(
        matches!(err, ProbeError::Corrupt { .. } | ProbeError::Incomplete),
        "{err:?}"
    );
}

/// M2-MEDIA-09: a media container without audio or video is `NoStreams`.
#[test]
fn f4_container_without_av_is_no_streams() {
    require_ffmpeg_cli!();
    let srt = support::dir().join("f4.srt");
    std::fs::write(&srt, "1\n00:00:00,000 --> 00:00:01,000\nhello\n").unwrap();
    let mkv = cli(
        "f4_subs_only.mkv",
        &["-i", srt.to_str().unwrap(), "-c:s", "srt"],
    );
    let err = probe(&mkv).unwrap_err();
    assert!(
        matches!(err, ProbeError::NoStreams),
        "subtitle-only MKV: {err:?}"
    );
}

/// M2-MEDIA-09: an M4A holding only cover art is `NoStreams`.
#[test]
fn f4b_cover_art_only_m4a_is_no_streams() {
    require_ffmpeg_cli!();
    let png = support::cached("f4b_cover.png", |p| support::png_with_alpha(p, 32, 32));
    let m4a = cli(
        "f4b_cover_only.m4a",
        &[
            "-i",
            png.to_str().unwrap(),
            "-map",
            "0",
            "-c",
            "copy",
            "-disposition:v:0",
            "attached_pic",
        ],
    );
    let err = probe(&m4a).unwrap_err();
    assert!(
        matches!(err, ProbeError::NoStreams),
        "cover-only M4A: {err:?}"
    );
}

/// M2-MEDIA-10: in a seek-unreliable container, a forward step beyond the
/// forward window decodes on instead of restarting from the beginning.
#[test]
fn m1_mp3_forward_jump_does_not_restart_from_zero() {
    require_ffmpeg_cli!();
    let p = cli(
        "m1_long.mp3",
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=441:sample_rate=44100",
            "-t",
            "300",
            "-c:a",
            "libmp3lame",
            "-q:a",
            "6",
        ],
    );
    let mut r = AudioReader::open(&p, 0, SampleRate::new(44_100).unwrap(), 1).unwrap();
    let t0 = std::time::Instant::now();
    let first = r.read(Tick::from_secs(240).unwrap(), 4410).unwrap();
    let first_cost = t0.elapsed();
    let t1 = std::time::Instant::now();
    let second = r.read(Tick::from_secs(246).unwrap(), 4410).unwrap();
    let second_cost = t1.elapsed();
    assert!(
        second_cost < first_cost / 4,
        "6 s forward step re-decoded from the start ({second_cost:?} vs {first_cost:?})"
    );
    // The same reads from a fresh reader give the same samples.
    let mut fresh = AudioReader::open(&p, 0, SampleRate::new(44_100).unwrap(), 1).unwrap();
    assert_eq!(
        fresh.read(Tick::from_secs(240).unwrap(), 4410).unwrap(),
        first
    );
    let mut fresh = AudioReader::open(&p, 0, SampleRate::new(44_100).unwrap(), 1).unwrap();
    assert_eq!(
        fresh.read(Tick::from_secs(246).unwrap(), 4410).unwrap(),
        second
    );
}

/// M2-MEDIA-11: a frame count whose sample count overflows is an error,
/// not a panic.
#[test]
fn f5_huge_read_does_not_panic() {
    let p = support::cached("f5.wav", |p| {
        support::write_wav(p, 48_000, 2, 4800);
    });
    let mut r = AudioReader::open(&p, 0, SampleRate::new(48_000).unwrap(), 2).unwrap();
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        r.read(Tick::ZERO, usize::MAX / 2 + 1)
    }));
    assert!(
        res.is_ok(),
        "AudioReader::read panicked on a huge frame count"
    );
    assert!(matches!(
        res.unwrap(),
        Err(freecut_media::DecodeError::InvalidArgument(_))
    ));
}

/// A plain-PCM WAV (`WAVE_FORMAT_PCM`, so no channel mask) of `channels`
/// 16-bit channels at 48 kHz, 4800 frames, with a full-scale square wave
/// on channel `loud` only.
fn unlabelled_wav(channels: u16, loud: u16) -> Vec<u8> {
    let frames = 4800u32;
    let block = u32::from(channels) * 2;
    let data_len = frames * block;
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // WAVE_FORMAT_PCM
    b.extend_from_slice(&channels.to_le_bytes());
    b.extend_from_slice(&48_000u32.to_le_bytes());
    b.extend_from_slice(&(48_000 * block).to_le_bytes());
    b.extend_from_slice(&(block as u16).to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..frames {
        for ch in 0..channels {
            let v: i16 = if ch == loud {
                if (i / 24) % 2 == 0 { 16_000 } else { -16_000 }
            } else {
                0
            };
            b.extend_from_slice(&v.to_le_bytes());
        }
    }
    b
}

/// R6: an unlabelled multichannel stream is mixed to stereo without
/// dropping a channel (FFmpeg's default layout calls channel 3 of 3 and
/// channel 4 of 6 or 8 "LFE").
#[test]
fn unlabelled_wav_channels_are_all_audible_in_stereo() {
    for (channels, loud) in [(3u16, 2u16), (6, 3), (8, 3), (4, 3)] {
        let name = format!("r6_{channels}ch_{loud}.wav");
        let p = write(&name, &unlabelled_wav(channels, loud));
        let info = probe(&p).unwrap().info;
        assert_eq!(info.audio[0].channels, channels);
        let mut r = AudioReader::open(&p, 0, SampleRate::new(48_000).unwrap(), 2).unwrap();
        let out = r.read(Tick::ZERO, 4800).unwrap();
        let peak = |side: usize| {
            out.iter()
                .skip(side)
                .step_by(2)
                .fold(0f32, |m, v| m.max(v.abs()))
        };
        let (l, r) = (peak(0), peak(1));
        assert!(
            l.max(r) > 0.3,
            "{channels} ch, signal on #{loud}: peak L {l:.3} R {r:.3}"
        );
    }
}

/// A 64 × 64 10-bit H.264 frame of flat grey `0xVVVVVV` tagged with
/// primaries / transfer / matrix `(prim, trc, matrix)` (H.273 codes).
fn tagged_grey(name: &str, rgb: &str, tags: (u8, u8, u8)) -> PathBuf {
    let (prim, trc, matrix) = tags;
    let bsf = format!(
        "h264_metadata=colour_primaries={prim}:transfer_characteristics={trc}:matrix_coefficients={matrix}"
    );
    let colour = format!("color=c=0x{rgb}:s=64x64:d=0.2");
    cli(
        name,
        &[
            "-f",
            "lavfi",
            "-i",
            &colour,
            "-vf",
            "scale=out_color_matrix=bt2020nc:out_range=tv",
            "-pix_fmt",
            "yuv420p10le",
            "-c:v",
            "libx264",
            "-bsf:v",
            &bsf,
        ],
    )
}

/// R1 / CL-002: HDR sources are tone-mapped to SDR Rec.709. HLG reference
/// white (75 % signal) lands at about 90 % (it came out at 75 %, code 191,
/// when passed through unconverted), and PQ 203 cd/m² does the same.
#[test]
fn hdr_reference_white_is_tone_mapped_to_about_90_percent() {
    require_ffmpeg_cli!();
    // 75 % HLG: 0xBF; PQ 203 cd/m² is 58 % signal: 0x94.
    for (name, rgb, trc) in [("r1_hlg.mp4", "BFBFBF", 18), ("r1_pq.mp4", "949494", 16)] {
        let path = tagged_grey(name, rgb, (9, trc, 9));
        let mut dec = VideoDecoder::open(&path, 0).unwrap();
        let frame = dec.frame_at(Tick::ZERO, SeekMode::Exact).unwrap();
        let px = &frame.rgba[(32 * 64 + 32) * 4..][..4];
        assert!(
            px[..3].iter().all(|&c| (222..=236).contains(&c)) && px[3] == 255,
            "{name}: {px:?}"
        );
    }
}

/// CL-002: a poster says when its source is HDR (PQ or HLG) and was
/// converted to SDR, so the app can tell the user once; SDR with BT.2020
/// primaries isn't HDR.
#[test]
fn posters_report_an_hdr_source() {
    require_ffmpeg_cli!();
    for (name, trc, hdr) in [
        ("r1_poster_hlg.mp4", 18, Some(HdrTransfer::Hlg)),
        ("r1_poster_pq.mp4", 16, Some(HdrTransfer::Pq)),
        ("r1_poster_sdr2020.mp4", 1, None),
    ] {
        let path = tagged_grey(name, "808080", (9, trc, 9));
        assert_eq!(freecut_media::poster(&path, 64).unwrap().hdr, hdr, "{name}");
    }
}

/// CL-002: SDR with BT.2020 primaries is gamut-mapped only: grey stays
/// grey.
#[test]
fn bt2020_sdr_grey_is_unchanged() {
    require_ffmpeg_cli!();
    let path = tagged_grey("r1_sdr2020.mp4", "808080", (9, 1, 9));
    let mut dec = VideoDecoder::open(&path, 0).unwrap();
    let frame = dec.frame_at(Tick::ZERO, SeekMode::Exact).unwrap();
    let px = &frame.rgba[(32 * 64 + 32) * 4..][..4];
    assert!(px[..3].iter().all(|&c| c.abs_diff(128) <= 2), "{px:?}");
}
