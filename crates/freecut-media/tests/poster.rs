//! Posters: time choice, sizing, pixel aspect and display orientation.
#![cfg(feature = "ffmpeg")]
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod support;

use std::path::PathBuf;

use freecut_media::{DecodeError, StreamKind, poster};
use support::{Pattern, VideoSpec, nearest_quadrant_color, quadrant_means, read_barcode};

fn barcode(name: &str, frames: u32) -> PathBuf {
    support::cached(name, |p| {
        support::encode_video(p, &VideoSpec::barcode(frames, (25, 1)));
    })
}

fn quadrants() -> PathBuf {
    support::cached("poster_quadrants.mp4", |p| {
        let mut spec = VideoSpec::barcode(25, (25, 1));
        spec.pattern = Pattern::Quadrants;
        spec.width = 160;
        spec.height = 96;
        support::encode_video(p, &spec);
    })
}

#[test]
fn poster_time_is_ten_percent_capped_at_one_second() {
    // 4.8 s: 10 % is 0.48 s = frame 12.
    let img = poster(&barcode("p120.mp4", 120), 192).unwrap();
    assert_eq!((img.width, img.height), (192, 144));
    assert_eq!(read_barcode(&img.rgba, img.width, img.height), 12);
    // 12 s: capped at 1 s = frame 25.
    let img = poster(&barcode("p300.mp4", 300), 192).unwrap();
    assert_eq!(read_barcode(&img.rgba, img.width, img.height), 25);
}

#[test]
fn short_clips_show_their_first_frame() {
    let img = poster(&barcode("p10.mp4", 10), 192).unwrap();
    assert_eq!(read_barcode(&img.rgba, img.width, img.height), 0);
}

#[test]
fn posters_fit_max_side_and_never_upscale() {
    let path = barcode("p120.mp4", 120);
    let img = poster(&path, 96).unwrap();
    assert_eq!((img.width, img.height), (96, 72));
    assert_eq!(img.rgba.len(), 96 * 72 * 4);
    assert_eq!(read_barcode(&img.rgba, img.width, img.height), 12);
    let img = poster(&path, 1000).unwrap();
    assert_eq!((img.width, img.height), (192, 144));
    assert!(matches!(
        poster(&path, 0),
        Err(DecodeError::InvalidArgument(_))
    ));
}

#[test]
fn pixel_aspect_is_applied() {
    let path = support::cached("poster_sar43.mp4", |p| {
        let mut spec = VideoSpec::barcode(10, (25, 1));
        spec.sar = Some((4, 3));
        support::encode_video(p, &spec);
    });
    let img = poster(&path, 256).unwrap();
    assert_eq!((img.width, img.height), (256, 144));
    let img = poster(&path, 128).unwrap();
    assert_eq!((img.width, img.height), (128, 72));
}

fn decode_png(path: &std::path::Path) -> (Vec<u8>, u32, u32) {
    let img = image::open(path).unwrap().to_rgba8();
    let (w, h) = img.dimensions();
    (img.into_raw(), w, h)
}

#[test]
fn orientation_matches_ffmpeg_autorotation() {
    require_ffmpeg_cli!();
    let src = quadrants();
    let cases: [(&str, &[&str]); 6] = [
        ("orient_rot90.mp4", &["-display_rotation:v:0", "90"]),
        ("orient_rot270.mp4", &["-display_rotation:v:0", "-90"]),
        ("orient_rot180.mp4", &["-display_rotation:v:0", "180"]),
        ("orient_hflip.mp4", &["-display_hflip:v:0"]),
        ("orient_vflip.mp4", &["-display_vflip:v:0"]),
        (
            "orient_rot90_hflip.mp4",
            &["-display_rotation:v:0", "90", "-display_hflip:v:0"],
        ),
    ];
    for (name, args) in cases {
        let path = support::dir().join(name);
        support::remux_with(&src, &path, args);
        let reference = support::dir().join(format!("{name}.png"));
        support::run_ffmpeg(&[
            "-i",
            path.to_str().unwrap(),
            "-frames:v",
            "1",
            "-update",
            "1",
            reference.to_str().unwrap(),
        ]);
        let (want, ww, wh) = decode_png(&reference);
        let img = poster(&path, 512).unwrap();
        assert_eq!((img.width, img.height), (ww, wh), "{name}: size");
        let ours = quadrant_means(&img.rgba, img.width, img.height).map(nearest_quadrant_color);
        let theirs = quadrant_means(&want, ww, wh).map(nearest_quadrant_color);
        assert_eq!(ours, theirs, "{name}: quadrant colors");
    }
}

#[test]
fn still_posters_keep_alpha_and_apply_exif_orientation() {
    let png = support::cached("poster_alpha.png", |p| support::png_with_alpha(p, 64, 48));
    let img = poster(&png, 256).unwrap();
    let (want, w, h) = decode_png(&png);
    assert_eq!((img.width, img.height), (w, h));
    assert_eq!(img.rgba, want, "an unscaled PNG poster is the image itself");
    assert!(img.rgba.chunks(4).any(|p| p[3] < 255));

    let jpg = support::cached("poster_exif6.jpg", |p| {
        support::jpeg_with_orientation(p, 64, 32, 6);
    });
    let img = poster(&jpg, 256).unwrap();
    assert_eq!((img.width, img.height), (32, 64));
    // Rotated clockwise: coded bottom-left (blue) is now top-left, etc.
    let q = quadrant_means(&img.rgba, img.width, img.height).map(nearest_quadrant_color);
    assert_eq!(q, [2, 0, 3, 1]);
}

#[test]
fn audio_only_files_have_no_poster() {
    let wav = support::cached("poster_tone.wav", |p| {
        support::write_wav(p, 48_000, 1, 4800);
    });
    assert!(matches!(
        poster(&wav, 128),
        Err(DecodeError::NoSuchStream {
            kind: StreamKind::Video,
            ..
        })
    ));
}
