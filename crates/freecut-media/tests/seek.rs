//! Frame-accurate video decoding against barcode fixtures.
#![cfg(feature = "ffmpeg")]
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod support;

use std::path::{Path, PathBuf};

use freecut_media::{DecodeError, SeekMode, StreamKind, VideoDecoder};
use freecut_timeline::time::{FrameRate, Tick};
use support::{Rng, VideoSpec, read_barcode};

fn h264_25() -> PathBuf {
    support::cached("barcode_25.mp4", |p| {
        support::encode_video(p, &VideoSpec::barcode(120, (25, 1)));
    })
}

fn h264_2997() -> PathBuf {
    support::cached("barcode_2997.mp4", |p| {
        support::encode_video(p, &VideoSpec::barcode(150, (30_000, 1001)));
    })
}

fn h264_2997_15360() -> PathBuf {
    support::cached("barcode_2997_15360.mp4", |p| {
        let mut spec = VideoSpec::barcode(120, (30_000, 1001));
        spec.stream_time_base = Some((1, 15_360));
        support::encode_video(p, &spec);
    })
}

fn mkv_25() -> PathBuf {
    support::cached("barcode_25.mkv", |p| {
        support::encode_video(p, &VideoSpec::barcode(60, (25, 1)));
    })
}

/// Index of the frame displayed at `t` per TR-036: the last start `<= t`,
/// or the first frame before it.
fn expected(starts: &[Tick], t: Tick) -> usize {
    starts.partition_point(|s| *s <= t).saturating_sub(1)
}

fn check(dec: &mut VideoDecoder, starts: &[Tick], t: Tick, mode: SeekMode) {
    let f = dec.frame_at(t, mode).unwrap();
    let k = read_barcode(&f.rgba, f.width, f.height) as usize;
    let want = expected(starts, t);
    assert_eq!(k, want, "frame at {t:?} ({mode:?})");
    assert_eq!(f.pts, starts[k], "pts of frame {k}");
}

fn hash(rgba: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    rgba.hash(&mut h);
    h.finish()
}

/// Full-picture hashes of every frame from one sequential walk.
fn sequential_hashes(path: &Path, starts: &[Tick]) -> Vec<u64> {
    let mut dec = VideoDecoder::open(path, 0).unwrap();
    starts
        .iter()
        .map(|s| hash(&dec.frame_at(*s, SeekMode::Exact).unwrap().rgba))
        .collect()
}

/// 220 random targets in random order, plus every boundary shuffled. Each
/// answer shows the expected barcode and is bit-identical to the same frame
/// reached sequentially (no decoder shortcut changes pixels, §5.5).
fn random_access(path: &Path, seed: u64) {
    let starts = support::frame_starts(path);
    let hashes = sequential_hashes(path, &starts);
    let last = *starts.last().unwrap();
    let mut dec = VideoDecoder::open(path, 0).unwrap();
    let mut rng = Rng::new(seed);
    let mut targets: Vec<Tick> = (0..220)
        .map(|_| Tick::from_raw(rng.range(-50_000_000, last.raw() + 300_000_000)))
        .collect();
    for s in &starts {
        targets.push(*s);
        targets.push(Tick::from_raw(s.raw() - 1));
    }
    rng.shuffle(&mut targets);
    for t in targets {
        check(&mut dec, &starts, t, SeekMode::Exact);
        let f = dec.frame_at(t, SeekMode::Exact).unwrap();
        assert_eq!(
            hash(&f.rgba),
            hashes[expected(&starts, t)],
            "pixels at {t:?}"
        );
    }
}

#[test]
fn random_exact_seeks_at_25_fps() {
    random_access(&h264_25(), 0x2525);
}

#[test]
fn random_exact_seeks_at_29_97_fps() {
    random_access(&h264_2997(), 0x2997);
}

#[test]
fn random_exact_seeks_in_matroska() {
    random_access(&mkv_25(), 0x4b4d);
}

#[test]
fn sequential_walk_matches_every_frame() {
    let path = h264_25();
    let starts = support::frame_starts(&path);
    let mut dec = VideoDecoder::open(&path, 0).unwrap();
    for (k, s) in starts.iter().enumerate() {
        let f = dec.frame_at(*s, SeekMode::Exact).unwrap();
        assert_eq!(read_barcode(&f.rgba, f.width, f.height) as usize, k);
        if k + 1 < starts.len() {
            assert_eq!(f.duration, Some(starts[k + 1].checked_sub(*s).unwrap()));
        }
    }
}

#[test]
fn every_frame_boundary_of_a_15360_time_base() {
    let path = h264_2997_15360();
    let starts = support::frame_starts(&path);
    assert!(starts.len() >= 100);
    // 1/15360 s is 45937.5 ticks: starts are not on a whole-unit grid, so a
    // decoder comparing stream timestamps would show the previous frame.
    let mut dec = VideoDecoder::open(&path, 0).unwrap();
    for (k, s) in starts.iter().enumerate() {
        if k > 0 {
            check(
                &mut dec,
                &starts,
                Tick::from_raw(s.raw() - 1),
                SeekMode::Exact,
            );
        }
        check(&mut dec, &starts, *s, SeekMode::Exact);
        check(
            &mut dec,
            &starts,
            Tick::from_raw(s.raw() + 1),
            SeekMode::Exact,
        );
    }
    // The project frame grid at 29.97 maps onto the right frames too.
    let rate = FrameRate::new(30_000, 1001).unwrap();
    for k in 0..starts.len() as i64 {
        check(
            &mut dec,
            &starts,
            rate.frame_start(k).unwrap(),
            SeekMode::Exact,
        );
    }
    // And the same walk backwards (every step seeks).
    for s in starts.iter().rev() {
        check(&mut dec, &starts, *s, SeekMode::Exact);
    }
}

#[test]
fn before_the_first_and_after_the_last_frame() {
    let path = h264_25();
    let starts = support::frame_starts(&path);
    let mut dec = VideoDecoder::open(&path, 0).unwrap();
    let f = dec
        .frame_at(Tick::from_raw(-1_000_000_000), SeekMode::Exact)
        .unwrap();
    assert_eq!(read_barcode(&f.rgba, f.width, f.height), 0);
    let end = Tick::from_secs(3600).unwrap();
    let f = dec.frame_at(end, SeekMode::Exact).unwrap();
    assert_eq!(
        read_barcode(&f.rgba, f.width, f.height) as usize,
        starts.len() - 1
    );
    let f = dec.frame_at(Tick::MAX, SeekMode::Exact).unwrap();
    assert_eq!(
        read_barcode(&f.rgba, f.width, f.height) as usize,
        starts.len() - 1
    );
    // Back to the start after visiting the end.
    let f = dec.frame_at(Tick::ZERO, SeekMode::Exact).unwrap();
    assert_eq!(read_barcode(&f.rgba, f.width, f.height), 0);
    assert_eq!(dec.first_frame_time(), Tick::ZERO);
}

#[test]
fn approximate_mode_returns_a_frame_at_or_before_the_target() {
    let path = h264_25();
    let starts = support::frame_starts(&path);
    let mut dec = VideoDecoder::open(&path, 0).unwrap();
    let mut rng = Rng::new(7);
    for _ in 0..60 {
        let t = Tick::from_raw(rng.range(0, starts.last().unwrap().raw()));
        let f = dec.frame_at(t, SeekMode::Approximate).unwrap();
        let k = read_barcode(&f.rgba, f.width, f.height) as usize;
        assert!(
            k <= expected(&starts, t),
            "approximate frame after the target"
        );
        assert_eq!(f.pts, starts[k]);
        // An exact request right after still lands on the exact frame.
        check(&mut dec, &starts, t, SeekMode::Exact);
    }
}

#[test]
fn frames_are_rgba_at_the_coded_size() {
    let mut dec = VideoDecoder::open(&h264_25(), 0).unwrap();
    let f = dec.frame_at(Tick::ZERO, SeekMode::Exact).unwrap();
    assert_eq!((f.width, f.height), (192, 144));
    assert_eq!(f.rgba.len(), 192 * 144 * 4);
    assert!(f.rgba.chunks(4).all(|p| p[3] == 255));
}

#[test]
fn opening_a_non_video_stream_is_refused() {
    let err = VideoDecoder::open(&h264_25(), 5).unwrap_err();
    assert!(matches!(
        err,
        DecodeError::NoSuchStream {
            index: 5,
            kind: StreamKind::Video
        }
    ));
    let err = VideoDecoder::open(std::path::Path::new("/no/such/file.mp4"), 0).unwrap_err();
    assert!(matches!(err, DecodeError::Open(e) if e.is_not_found()));
}

#[test]
fn stills_decode_with_alpha_at_any_time() {
    let path = support::cached("seek_alpha.png", |p| support::png_with_alpha(p, 64, 48));
    let want = image::open(&path).unwrap().to_rgba8().into_raw();
    let mut dec = VideoDecoder::open(&path, 0).unwrap();
    for t in [0, 5_000_000_000, -1, 0] {
        let f = dec.frame_at(Tick::from_raw(t), SeekMode::Exact).unwrap();
        assert_eq!((f.width, f.height, f.pts), (64, 48, Tick::ZERO));
        assert_eq!(f.rgba, want, "at {t}");
    }
}
