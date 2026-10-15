//! Probing: MediaInfo snapshots per fixture and error mapping.
#![cfg(feature = "ffmpeg")]
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod support;

use std::path::PathBuf;

use freecut_media::{
    CancelToken, NotMediaKind, ProbeError, STILL_DURATION, fingerprint, init, probe,
    probe_with_cancel,
};
use freecut_timeline::model::{AudioStreamInfo, MediaInfo, Rotation, VideoStreamInfo};
use freecut_timeline::time::{FrameRate, Rational, TICKS_PER_SECOND, Tick};
use pretty_assertions::assert_eq;
use support::{AudioSpec, Pattern, VideoSpec};

const SECOND: i64 = TICKS_PER_SECOND;

fn av_clip() -> PathBuf {
    support::cached("probe_av.mp4", |p| {
        let mut spec = VideoSpec::barcode(75, (25, 1));
        spec.audio = Some(AudioSpec {
            rate: 48_000,
            channels: 2,
            samples: 48_000 * 3,
        });
        support::encode_video(p, &spec);
    })
}

fn quadrants() -> PathBuf {
    support::cached("quadrants.mp4", |p| {
        let mut spec = VideoSpec::barcode(25, (25, 1));
        spec.pattern = Pattern::Quadrants;
        spec.width = 160;
        spec.height = 96;
        support::encode_video(p, &spec);
    })
}

fn video(info: &MediaInfo) -> &VideoStreamInfo {
    &info.video[0]
}

#[test]
fn capabilities_report_the_core_codecs() {
    let caps = init().unwrap();
    assert_eq!(
        caps.version("avcodec").map(|v| v.major),
        Some(ffmpeg_next::ffi::LIBAVCODEC_VERSION_MAJOR)
    );
    assert!(caps.has_decoder("h264"));
    assert!(caps.has_decoder("aac"));
    assert!(caps.has_encoder("aac"));
    for name in ["hevc", "vp9", "av1", "prores", "mp3", "opus", "flac"] {
        assert!(
            caps.decoders.iter().any(|c| c.name == name),
            "{name} listed"
        );
    }
    for name in ["libx264", "libx265"] {
        assert!(
            caps.encoders.iter().any(|c| c.name == name),
            "{name} listed"
        );
    }
}

#[test]
fn h264_with_aac_snapshot() {
    let path = av_clip();
    let probed = probe(&path).unwrap();
    let expected = MediaInfo {
        container: "mov,mp4,m4a,3gp,3g2,mj2".into(),
        default_video: Some(0),
        duration: Tick::from_raw(3 * SECOND),
        video: vec![VideoStreamInfo {
            index: 0,
            codec: "h264".into(),
            width: 192,
            height: 144,
            sar: (1, 1),
            rotation: Rotation::None,
            mirrored: false,
            rate: Some(Rational::new(25, 1).unwrap()),
            variable_rate: false,
            has_alpha: false,
            bit_depth: 8,
            start: Tick::ZERO,
            duration: Tick::from_raw(3 * SECOND),
        }],
        audio: vec![AudioStreamInfo {
            index: 1,
            codec: "aac".into(),
            sample_rate: 48_000,
            channels: 2,
            start: Tick::ZERO,
            duration: Tick::from_raw(3 * SECOND),
            duration_exact: true,
            default: true,
            language: None,
            title: None,
            layout: Some("stereo".into()),
        }],
        still: None,
    };
    assert_eq!(probed.info, expected);
    assert_eq!(
        probed.fingerprint,
        fingerprint(&path, probed.info.duration).unwrap()
    );
    assert_eq!(
        probed.fingerprint.size,
        std::fs::metadata(&path).unwrap().len()
    );
}

#[test]
fn ntsc_rate_in_a_15360_time_base() {
    let path = support::cached("probe_2997_15360.mp4", |p| {
        let mut spec = VideoSpec::barcode(90, (30_000, 1001));
        spec.stream_time_base = Some((1, 15_360));
        support::encode_video(p, &spec);
    });
    let info = probe(&path).unwrap().info;
    let v = video(&info);
    assert_eq!(v.rate, Some(Rational::new(30_000, 1001).unwrap()));
    assert!(!v.variable_rate);
    // 90 frames; the 1/15360 time base is within one unit (45938 ticks).
    let frames = FrameRate::new(30_000, 1001).unwrap().frames(90).unwrap();
    assert!(
        v.duration.abs_diff(frames) <= 45_938,
        "{:?} vs {frames:?}",
        v.duration
    );
    assert_eq!(info.duration, v.duration);
}

#[test]
fn variable_frame_rate_is_flagged() {
    let path = support::cached("vfr.mp4", |p| {
        let mut spec = VideoSpec::barcode(60, (30, 1));
        // Alternating 1/30 s and 1/20 s frame intervals (1/90000 units).
        let mut pts = Vec::new();
        let mut t = 0i64;
        for k in 0..60 {
            pts.push(t);
            t += if k % 2 == 0 { 3000 } else { 4500 };
        }
        spec.pts = Some(pts);
        support::encode_video(p, &spec);
    });
    let info = probe(&path).unwrap().info;
    assert!(video(&info).variable_rate);

    // VFR frames are still delivered by timestamp.
    let starts = support::frame_starts(&path);
    let mut dec = freecut_media::VideoDecoder::open(&path, 0).unwrap();
    for (k, s) in starts.iter().enumerate().rev() {
        let f = dec.frame_at(*s, freecut_media::SeekMode::Exact).unwrap();
        assert_eq!(
            support::read_barcode(&f.rgba, f.width, f.height) as usize,
            k
        );
    }
}

#[test]
fn sample_aspect_ratio_is_reported() {
    let path = support::cached("sar43.mp4", |p| {
        let mut spec = VideoSpec::barcode(10, (25, 1));
        spec.sar = Some((4, 3));
        support::encode_video(p, &spec);
    });
    assert_eq!(video(&probe(&path).unwrap().info).sar, (4, 3));
}

#[test]
fn rotation_and_mirroring_from_the_display_matrix() {
    require_ffmpeg_cli!();
    let src = quadrants();
    // `-display_rotation` is counter-clockwise; Rotation is clockwise.
    let cases: [(&str, &[&str], Rotation, bool); 5] = [
        (
            "rot90.mp4",
            &["-display_rotation:v:0", "90"],
            Rotation::Cw270,
            false,
        ),
        (
            "rot270.mp4",
            &["-display_rotation:v:0", "-90"],
            Rotation::Cw90,
            false,
        ),
        (
            "rot180.mp4",
            &["-display_rotation:v:0", "180"],
            Rotation::Cw180,
            false,
        ),
        ("hflip.mp4", &["-display_hflip:v:0"], Rotation::None, true),
        (
            "rot90_hflip.mp4",
            &["-display_rotation:v:0", "90", "-display_hflip:v:0"],
            Rotation::Cw90,
            true,
        ),
    ];
    for (name, args, rotation, mirrored) in cases {
        let path = support::dir().join(name);
        support::remux_with(&src, &path, args);
        let v = video(&probe(&path).unwrap().info).clone();
        assert_eq!((v.rotation, v.mirrored), (rotation, mirrored), "{name}");
        // Coded size is unchanged by the matrix.
        assert_eq!((v.width, v.height), (160, 96), "{name}");
    }
}

#[test]
fn png_still_with_alpha() {
    let path = support::cached("alpha.png", |p| support::png_with_alpha(p, 64, 48));
    let info = probe(&path).unwrap().info;
    assert_eq!(info.still, Some((64, 48)));
    assert_eq!(info.duration, STILL_DURATION);
    assert!(info.audio.is_empty());
    let v = video(&info);
    assert_eq!(v.codec, "png");
    assert!(v.has_alpha);
    assert_eq!(v.rate, None);
    assert_eq!((v.start, v.duration), (Tick::ZERO, STILL_DURATION));
}

#[test]
fn jpeg_still_with_exif_orientation() {
    let path = support::cached("exif6.jpg", |p| {
        support::jpeg_with_orientation(p, 64, 32, 6);
    });
    let info = probe(&path).unwrap().info;
    let v = video(&info);
    assert_eq!(v.codec, "mjpeg");
    assert_eq!((v.width, v.height), (64, 32));
    assert_eq!((v.rotation, v.mirrored), (Rotation::Cw90, false));
    // The still size is the displayed size.
    assert_eq!(info.still, Some((32, 64)));
    assert!(!v.has_alpha);
}

#[test]
fn audio_only_files() {
    let path = support::cached("probe_tone.wav", |p| {
        support::write_wav(p, 44_100, 1, 44_100 * 2);
    });
    let info = probe(&path).unwrap().info;
    assert!(info.video.is_empty());
    assert_eq!(info.duration, Tick::from_raw(2 * SECOND));
    let a = &info.audio[0];
    assert_eq!(
        (a.codec.as_str(), a.sample_rate, a.channels),
        ("pcm_s16le", 44_100, 1)
    );
    assert_eq!(
        (a.start, a.duration, a.duration_exact),
        (Tick::ZERO, Tick::from_raw(2 * SECOND), true)
    );
}

fn write(name: &str, bytes: &[u8]) -> PathBuf {
    let p = support::dir().join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

#[test]
fn empty_missing_and_folder_paths() {
    assert!(matches!(
        probe(&write("empty.mp4", b"")),
        Err(ProbeError::Empty)
    ));
    let missing = probe(std::path::Path::new("/no/such/dir/clip.mp4")).unwrap_err();
    assert!(missing.is_not_found(), "{missing:?}");
    assert!(matches!(
        probe(support::dir()),
        Err(ProbeError::UnsupportedPath { .. })
    ));
}

#[test]
fn random_bytes_and_known_non_media() {
    let mut rng = support::Rng::new(99);
    let noise: Vec<u8> = (0..65_536).map(|_| rng.next_u64() as u8).collect();
    assert!(matches!(
        probe(&write("noise.mp4", &noise)),
        Err(ProbeError::NotMedia {
            kind: NotMediaKind::Unrecognized
        })
    ));
    assert!(matches!(
        probe(&write("archive.mov", b"PK\x03\x04\x14\x00\x00\x00")),
        Err(ProbeError::NotMedia {
            kind: NotMediaKind::Archive
        })
    ));
    assert!(matches!(
        probe(&write("project.mp4", br#"{"freecut": {"version": 1}}"#)),
        Err(ProbeError::NotMedia {
            kind: NotMediaKind::Project
        })
    ));
}

#[test]
fn truncated_mp4_without_index_is_incomplete() {
    let full = std::fs::read(av_clip()).unwrap();
    // The fixture writes `moov` at the end; half the file has none.
    let path = write("truncated.mp4", &full[..full.len() / 2]);
    assert!(matches!(probe(&path), Err(ProbeError::Incomplete)));
}

#[test]
fn garbage_in_the_media_data_is_corrupt() {
    let mut bytes = std::fs::read(quadrants()).unwrap();
    // Overwrite the payload of the `mdat` box with noise.
    let at = bytes.windows(4).position(|w| w == b"mdat").unwrap();
    let size = u32::from_be_bytes(bytes[at - 4..at].try_into().unwrap()) as usize;
    let mut rng = support::Rng::new(5);
    for b in &mut bytes[at + 4..at - 4 + size] {
        *b = rng.next_u64() as u8;
    }
    let err = probe(&write("garbage.mp4", &bytes)).unwrap_err();
    assert!(matches!(err, ProbeError::Corrupt { .. }), "{err:?}");
}

#[test]
fn a_cancelled_probe_stops() {
    let token = CancelToken::new();
    token.cancel();
    assert!(matches!(
        probe_with_cancel(&av_clip(), &token),
        Err(ProbeError::Cancelled)
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn non_utf8_paths_open_without_panicking() {
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::OsStr::from_bytes(b"clip-\xff\xfe.mp4");
    let path = support::dir().join(name);
    std::fs::copy(av_clip(), &path).unwrap();
    let info = probe(&path).unwrap().info;
    assert_eq!(info.video.len(), 1);
}

#[test]
fn a_codec_without_a_decoder_is_unsupported() {
    require_ffmpeg_cli!();
    // An AVI whose video FourCC no decoder knows.
    let path = support::dir().join("unknown_fourcc.avi");
    support::run_ffmpeg(&[
        "-i",
        quadrants().to_str().unwrap(),
        "-c:v",
        "copy",
        "-vtag",
        "QQQQ",
        "-strict",
        "unofficial",
        path.to_str().unwrap(),
    ]);
    let err = probe(&path).unwrap_err();
    assert!(
        matches!(&err, ProbeError::UnsupportedCodec { codec } if codec == "QQQQ"),
        "{err:?}"
    );
}

#[test]
fn several_audio_streams_report_default_language_title_and_layout() {
    require_ffmpeg_cli!();
    let path = support::dir().join("two_streams.mkv");
    support::run_ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "sine=f=440:r=48000:d=1",
        "-f",
        "lavfi",
        "-i",
        "aevalsrc=0.1|0.1|0.1|0.1|0.1|0.1:c=5.1(side):s=48000:d=1",
        "-map",
        "0:a",
        "-map",
        "1:a",
        "-c:a",
        "flac",
        "-metadata:s:a:0",
        "language=eng",
        "-metadata:s:a:0",
        "title=Camera mic",
        "-metadata:s:a:1",
        "language=und",
        "-disposition:a:0",
        "0",
        "-disposition:a:1",
        "default",
        path.to_str().unwrap(),
    ]);
    let info = probe(&path).unwrap().info;
    let streams: Vec<_> = info
        .audio
        .iter()
        .map(|a| {
            (
                a.index,
                a.channels,
                a.default,
                a.language.as_deref(),
                a.title.as_deref(),
                a.layout.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        streams,
        [
            (0, 1, false, Some("eng"), Some("Camera mic"), Some("mono")),
            // "und" (undetermined) is no language.
            (1, 6, true, None, None, Some("5.1(side)")),
        ]
    );
    assert_eq!(info.default_audio().map(|a| a.index), Some(1));
}
