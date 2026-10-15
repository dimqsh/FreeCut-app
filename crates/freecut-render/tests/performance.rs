//! Repeatable CPU-only benchmark (no decoding or GUI). Run with:
//! `cargo test -p freecut-render --release --test performance -- --ignored --nocapture`
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::hint::black_box;
use std::time::{Duration, Instant};

use common::*;
use freecut_render::{Compositor, FontRegistry, Frame};
use freecut_timeline::glam::{DMat3, DVec2};

#[test]
#[ignore = "benchmark"]
fn cpu_frames() {
    for (name, size, transform, count) in [
        ("tile", (176, 132), DMat3::IDENTITY, 1),
        ("preview", (960, 540), DMat3::IDENTITY, 1),
        ("1080p", (1920, 1080), DMat3::IDENTITY, 1),
        ("4k", (3840, 2160), DMat3::IDENTITY, 1),
        (
            "rotated-preview",
            (960, 540),
            DMat3::from_translation(DVec2::new(80.25, 20.5)) * DMat3::from_angle(0.1),
            3,
        ),
    ] {
        let (w, h) = size;
        let mut images = Images::default().with(1, pattern(w, h));
        let plan = plan(
            w,
            h,
            (0..count)
                .map(|_| layer(still(1), size, transform))
                .collect(),
        );
        for threads in [1, 4] {
            let mut compositor = Compositor::new(FontRegistry::bundled()).with_threads(threads);
            let mut frame = Frame::new(w, h);
            for _ in 0..3 {
                compositor.render_into(&plan, &mut frame, &mut images);
            }
            let start = Instant::now();
            let mut runs = 0u32;
            while start.elapsed() < Duration::from_millis(500) {
                compositor.render_into(black_box(&plan), black_box(&mut frame), &mut images);
                black_box(&frame);
                runs += 1;
            }
            let ms = start.elapsed().as_secs_f64() * 1_000.0 / f64::from(runs);
            let checksum: u64 = frame.rgba.iter().map(|&b| u64::from(b)).sum();
            println!("{name:>16} {w}x{h} threads={threads} ms/frame={ms:.3} checksum={checksum}");
        }
    }
}
