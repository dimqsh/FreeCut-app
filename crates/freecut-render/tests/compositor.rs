//! The CPU compositor: placement through `layer_to_canvas`, display
//! orientation, blending, output scaling, determinism.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::f64::consts::FRAC_PI_2;
use std::time::Instant;

use common::*;
use freecut_render::cpu::{
    Compositor, Frame, NoImages, PLACEHOLDER, Picture, SLATE, TextAnchor, TextLayer,
};
use freecut_render::fit_size;
use freecut_render::text::{Fill, FontRegistry, TextSpec};
use std::sync::Arc;

use freecut_timeline::glam::{DMat3, DVec2};
use freecut_timeline::model::{GeneratedKind, Rotation, TextClipData};
use freecut_timeline::plan::{LayerSource, fit_transform};
use freecut_timeline::time::Tick;
use pretty_assertions::assert_eq;

fn compositor() -> Compositor {
    Compositor::new(FontRegistry::bundled())
}

fn rows(frame: &Frame) -> Vec<Vec<[u8; 4]>> {
    (0..frame.height)
        .map(|y| {
            (0..frame.width)
                .map(|x| frame.pixel(x, y).unwrap())
                .collect()
        })
        .collect()
}

/// A text layer as the planner makes it (the provider supplies the text).
fn text_layer(canvas: (u32, u32), m: DMat3) -> freecut_timeline::plan::LayerPlan {
    layer(
        LayerSource::Text {
            data: Arc::new(TextClipData::default()),
            scale_px: 1.0,
        },
        canvas,
        m,
    )
}

fn translate(x: f64, y: f64) -> DMat3 {
    DMat3::from_translation(DVec2::new(x, y))
}

#[test]
fn an_empty_plan_is_the_background() {
    let frame = compositor().render(&plan(4, 3, vec![]), 4, 3, &mut NoImages);
    assert_eq!(frame.rgba.len(), 4 * 3 * 4);
    assert!(frame.rgba.chunks(4).all(|p| p == BG));
}

#[test]
fn an_identity_layer_copies_the_picture_exactly() {
    let pic = pattern(16, 9);
    let mut images = Images::default().with(1, pic.clone());
    let p = plan(16, 9, vec![layer(still(1), (16, 9), DMat3::IDENTITY)]);
    let frame = compositor().render(&p, 16, 9, &mut images);
    // Opaque texels are copied; translucent ones blend over black.
    for (got, want) in frame.rgba.chunks(4).zip(pic.rgba.chunks(4)) {
        if want[3] == 255 {
            assert_eq!(got, want);
        } else {
            assert_eq!(got[3], 255);
        }
    }
}

#[test]
fn fit_letterboxes_a_wide_picture() {
    let mut images = Images::default().with(1, solid(2, 1, RED));
    let p = plan(
        4,
        4,
        vec![layer(still(1), (2, 1), fit_transform((2, 1), (4, 4)))],
    );
    let frame = compositor().render(&p, 4, 4, &mut images);
    assert_eq!(
        rows(&frame),
        vec![vec![BG; 4], vec![RED; 4], vec![RED; 4], vec![BG; 4],]
    );
}

#[test]
fn layers_stack_bottom_to_top_at_their_positions() {
    let mut images = Images::default()
        .with(1, solid(2, 2, RED))
        .with(2, solid(1, 1, GREEN));
    let p = plan(
        3,
        2,
        vec![
            layer(still(1), (2, 2), DMat3::IDENTITY),
            layer(still(2), (1, 1), translate(1.0, 1.0)),
        ],
    );
    let frame = compositor().render(&p, 3, 2, &mut images);
    assert_eq!(rows(&frame), vec![vec![RED, RED, BG], vec![RED, GREEN, BG]]);
}

#[test]
fn a_quarter_turn_transform_rotates_the_layer() {
    let mut images = Images::default().with(1, picture(&[&[RED, GREEN]]));
    // Clockwise on screen (y down), then moved back into view.
    let m = translate(1.0, 0.0) * DMat3::from_angle(FRAC_PI_2);
    let p = plan(1, 2, vec![layer(still(1), (2, 1), m)]);
    let frame = compositor().render(&p, 1, 2, &mut images);
    assert_eq!(rows(&frame), vec![vec![RED], vec![GREEN]]);
}

#[test]
fn a_negative_scale_flips_the_layer() {
    let mut images = Images::default().with(1, picture(&[&[RED, GREEN, BLUE]]));
    let flip_h = translate(3.0, 0.0) * DMat3::from_scale(DVec2::new(-1.0, 1.0));
    let p = plan(3, 1, vec![layer(still(1), (3, 1), flip_h)]);
    let frame = compositor().render(&p, 3, 1, &mut images);
    assert_eq!(rows(&frame), vec![vec![BLUE, GREEN, RED]]);

    let mut images = Images::default().with(1, picture(&[&[RED], &[GREEN]]));
    let flip_v = translate(0.0, 2.0) * DMat3::from_scale(DVec2::new(1.0, -1.0));
    let p = plan(1, 2, vec![layer(still(1), (1, 2), flip_v)]);
    let frame = compositor().render(&p, 1, 2, &mut images);
    assert_eq!(rows(&frame), vec![vec![GREEN], vec![RED]]);
}

#[test]
fn display_rotation_and_mirroring_follow_the_media_convention() {
    // Stored 2 × 1 [RED, GREEN]; displayed 1 × 2.
    let cases = [
        (Rotation::Cw90, false, vec![vec![RED], vec![GREEN]]),
        (Rotation::Cw270, false, vec![vec![GREEN], vec![RED]]),
        (Rotation::Cw90, true, vec![vec![GREEN], vec![RED]]),
    ];
    for (rotation, mirrored, want) in cases {
        let mut pic = picture(&[&[RED, GREEN]]);
        pic.rotation = rotation;
        pic.mirrored = mirrored;
        let mut images = Images::default().with(1, pic);
        let p = plan(1, 2, vec![layer(still(1), (1, 2), DMat3::IDENTITY)]);
        let frame = compositor().render(&p, 1, 2, &mut images);
        assert_eq!(rows(&frame), want, "{rotation:?} mirrored={mirrored}");
    }
    // Upright cases keep the stored size.
    let cases = [
        (Rotation::Cw180, false, vec![vec![GREEN, RED]]),
        (Rotation::None, true, vec![vec![GREEN, RED]]),
        (Rotation::Cw180, true, vec![vec![RED, GREEN]]),
    ];
    for (rotation, mirrored, want) in cases {
        let mut pic = picture(&[&[RED, GREEN]]);
        pic.rotation = rotation;
        pic.mirrored = mirrored;
        let mut images = Images::default().with(1, pic);
        let p = plan(2, 1, vec![layer(still(1), (2, 1), DMat3::IDENTITY)]);
        let frame = compositor().render(&p, 2, 1, &mut images);
        assert_eq!(rows(&frame), want, "{rotation:?} mirrored={mirrored}");
    }
}

#[test]
fn non_square_pixels_are_stretched_to_the_display_size() {
    // A 1 × 1 stored picture shown 2 px wide (sample aspect 2:1).
    let mut images = Images::default().with(1, solid(1, 1, GREEN));
    let p = plan(2, 1, vec![layer(still(1), (2, 1), DMat3::IDENTITY)]);
    let frame = compositor().render(&p, 2, 1, &mut images);
    assert_eq!(rows(&frame), vec![vec![GREEN, GREEN]]);
}

#[test]
fn straight_alpha_blends_over_the_layers_below() {
    let half_white = [255, 255, 255, 128];
    let mut images = Images::default()
        .with(1, solid(1, 1, half_white))
        .with(2, solid(1, 1, RED));
    let p = plan(
        2,
        1,
        vec![
            layer(still(2), (1, 1), translate(1.0, 0.0)),
            layer(still(1), (2, 1), DMat3::IDENTITY),
        ],
    );
    let frame = compositor().render(&p, 2, 1, &mut images);
    // white·128/255 over black and over red.
    assert_eq!(
        rows(&frame),
        vec![vec![[128, 128, 128, 255], [255, 128, 128, 255]]]
    );
}

#[test]
fn layer_opacity_scales_alpha() {
    let mut images = Images::default().with(1, solid(1, 1, WHITE));
    let mut l = layer(still(1), (1, 1), DMat3::IDENTITY);
    l.opacity = 0.25;
    let frame = compositor().render(&plan(1, 1, vec![l.clone()]), 1, 1, &mut images);
    assert_eq!(frame.pixel(0, 0), Some([64, 64, 64, 255]));
    l.opacity = 0.0;
    let frame = compositor().render(&plan(1, 1, vec![l]), 1, 1, &mut images);
    assert_eq!(frame.pixel(0, 0), Some(BG));
}

#[test]
fn output_scaling_maps_the_canvas_onto_any_size() {
    // The left half of an 8 × 8 canvas is red.
    let mut images = Images::default().with(1, solid(4, 8, RED));
    let p = plan(8, 8, vec![layer(still(1), (4, 8), DMat3::IDENTITY)]);
    let small = compositor().render(&p, 4, 4, &mut images);
    assert_eq!(rows(&small), vec![vec![RED, RED, BG, BG]; 4]);
    let wide = compositor().render(&p, 16, 2, &mut images);
    let mut row = vec![RED; 8];
    row.extend(vec![BG; 8]);
    assert_eq!(rows(&wide), vec![row; 2]);
    // fit_size keeps the canvas aspect and never upscales.
    assert_eq!(fit_size((1920, 1080), (960, 960)), (960, 540));
    assert_eq!(fit_size((1920, 1080), (4000, 4000)), (1920, 1080));
    assert_eq!(fit_size((1080, 1920), (500, 500)), (281, 500));
}

#[test]
fn upscaling_interpolates_bilinearly() {
    let mut images = Images::default().with(1, picture(&[&[[0, 0, 0, 255], [200, 200, 200, 255]]]));
    let p = plan(2, 1, vec![layer(still(1), (2, 1), DMat3::IDENTITY)]);
    let frame = compositor().render(&p, 8, 1, &mut images);
    let reds: Vec<u8> = rows(&frame)[0].iter().map(|p| p[0]).collect();
    // Clamped at the outer quarter pixels, a ramp in between.
    assert_eq!(reds, vec![0, 0, 25, 75, 125, 175, 200, 200]);
}

#[test]
fn missing_pictures_draw_the_slate_and_generated_layers_a_placeholder() {
    let p = plan(
        3,
        1,
        vec![
            layer(still(9), (1, 1), DMat3::IDENTITY),
            layer(
                LayerSource::Slate { asset: media(3) },
                (1, 1),
                translate(1.0, 0.0),
            ),
            layer(
                LayerSource::Generated {
                    kind: GeneratedKind::Sticker,
                    local_time: Tick::ZERO,
                },
                (1, 1),
                translate(2.0, 0.0),
            ),
        ],
    );
    let mut bad = Picture::new(4, 4, vec![0; 3]);
    bad.rotation = Rotation::None;
    let mut images = Images::default().with(9, bad);
    let frame = compositor().render(&p, 3, 1, &mut images);
    let a = u32::from(PLACEHOLDER[3]);
    let placeholder: [u8; 4] = [0, 1, 2]
        .map(|c| ((u32::from(PLACEHOLDER[c]) * a + 127) / 255) as u8)
        .into_iter()
        .chain([255])
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    assert_eq!(rows(&frame), vec![vec![SLATE, SLATE, placeholder]]);
}

#[test]
fn video_layers_ask_for_their_source_time() {
    let t = Tick::from_raw(12_345);
    let mut images = Images::default().with(4, solid(1, 1, GREEN));
    let p = plan(
        1,
        1,
        vec![layer(
            LayerSource::Video {
                asset: media(4),
                source_time: t,
            },
            (1, 1),
            DMat3::IDENTITY,
        )],
    );
    let frame = compositor().render(&p, 1, 1, &mut images);
    assert_eq!(frame.pixel(0, 0), Some(GREEN));
    assert_eq!(images.video_requests, vec![(media(4), t)]);
}

#[test]
fn text_layers_are_rasterized_and_placed() {
    let text = |offset: f64| TextLayer {
        spec: TextSpec {
            fill: Fill::Solid(GREEN),
            size_px: 40.0,
            ..TextSpec::new("IIII")
        },
        layer_to_canvas: translate(offset, 10.0),
        anchor: TextAnchor::TopLeft,
        motion: None,
    };
    let generated = text_layer((200, 100), DMat3::IDENTITY);
    let p = plan(200, 100, vec![generated]);
    let mut images = Images::default();
    images.texts.insert(clip(1), text(20.0));
    let mut comp = compositor();
    let frame = comp.render(&p, 200, 100, &mut images);
    let green_columns: Vec<u32> = (0..200)
        .filter(|&x| (0..100).any(|y| frame.pixel(x, y) == Some(GREEN)))
        .collect();
    assert!(!green_columns.is_empty(), "no fully covered text pixels");
    assert!(green_columns[0] >= 20, "{green_columns:?}");
    // Everything outside the text block keeps the background.
    assert!((0..100).all(|y| frame.pixel(5, y) == Some(BG)));
    assert!((0..200).all(|x| frame.pixel(x, 2) == Some(BG)));

    // Moving the layer moves the pixels by the same amount.
    images.texts.insert(clip(1), text(60.0));
    let moved = comp.render(&p, 200, 100, &mut images);
    for y in 0..100 {
        for x in 0..140 {
            assert_eq!(frame.pixel(x, y), moved.pixel(x + 40, y), "({x}, {y})");
        }
    }

    // A provider that supplies no text draws nothing.
    let nothing = comp.render(&p, 200, 100, &mut Images::default());
    assert!(nothing.rgba.chunks(4).all(|p| p == BG));
}

#[test]
fn centre_anchored_text_is_centred_on_the_layer_origin() {
    // The plan's text convention: layer origin = centre of the text box.
    let p = plan(
        300,
        200,
        vec![text_layer((300, 200), translate(150.0, 100.0))],
    );
    let mut images = Images::default();
    images.texts.insert(
        clip(1),
        TextLayer {
            spec: TextSpec {
                fill: Fill::Solid(GREEN),
                size_px: 60.0,
                ..TextSpec::new("HH")
            },
            layer_to_canvas: translate(150.0, 100.0),
            anchor: TextAnchor::Center,
            motion: None,
        },
    );
    let frame = compositor().render(&p, 300, 200, &mut images);
    let inked: Vec<(u32, u32)> = (0..200)
        .flat_map(|y| (0..300).map(move |x| (x, y)))
        .filter(|&(x, y)| frame.pixel(x, y) != Some(BG))
        .collect();
    let (min_x, max_x) = (
        inked.iter().map(|p| p.0).min().unwrap(),
        inked.iter().map(|p| p.0).max().unwrap(),
    );
    // "HH" has equal side bearings: its ink is centred horizontally.
    let centre = f64::from(min_x + max_x + 1) / 2.0;
    assert!((centre - 150.0).abs() <= 1.5, "ink {min_x}..={max_x}");
    // Vertically the box (not the ink) is centred: caps sit above the
    // middle, between it and the top.
    let min_y = inked.iter().map(|p| p.1).min().unwrap();
    assert!(min_y > 50 && min_y < 100, "{min_y}");
}

#[test]
fn text_follows_the_output_scale() {
    let generated = text_layer((400, 200), DMat3::IDENTITY);
    let p = plan(400, 200, vec![generated]);
    let mut images = Images::default();
    images.texts.insert(
        clip(1),
        TextLayer {
            spec: TextSpec {
                size_px: 80.0,
                ..TextSpec::new("Hello")
            },
            layer_to_canvas: translate(40.0, 40.0),
            anchor: TextAnchor::TopLeft,
            motion: None,
        },
    );
    let mut comp = compositor();
    let full = comp.render(&p, 400, 200, &mut images);
    let half = comp.render(&p, 200, 100, &mut images);
    let ink = |f: &Frame| f.rgba.chunks(4).map(|p| u64::from(p[0])).sum::<u64>();
    // A quarter of the pixels carry about a quarter of the ink.
    let ratio = ink(&half) as f64 / ink(&full) as f64;
    assert!((0.22..0.28).contains(&ratio), "{ratio}");
}

#[test]
fn rotated_edges_are_antialiased_by_coverage() {
    // A 100 × 100 white square turned 30° about the centre of a black
    // 200 × 200 canvas.
    let mut images = Images::default().with(1, solid(100, 100, WHITE));
    let m = translate(100.0, 100.0)
        * DMat3::from_angle(std::f64::consts::FRAC_PI_6)
        * translate(-50.0, -50.0);
    let p = plan(200, 200, vec![layer(still(1), (100, 100), m)]);
    let frame = compositor().render(&p, 200, 200, &mut images);
    let reds: Vec<u8> = frame.rgba.chunks(4).map(|px| px[0]).collect();
    let partial = reds.iter().filter(|&&r| r > 0 && r < 255).count();
    // The perimeter is 400 px long: most pixels along it are partly
    // covered.
    assert!(partial > 300, "partial-coverage pixels: {partial}");
    // Coverage adds up to the square's area.
    let area: f64 = reds.iter().map(|&r| f64::from(r) / 255.0).sum();
    assert!((area - 10_000.0).abs() < 50.0, "area {area}");
}

#[test]
fn sub_pixel_offsets_give_partial_edge_pixels() {
    // A 4 × 1 white picture a quarter pixel to the right: the first pixel
    // is 3/4 covered, the one after the end 1/4.
    let mut images = Images::default().with(1, solid(4, 1, WHITE));
    let p = plan(6, 1, vec![layer(still(1), (4, 1), translate(0.25, 0.0))]);
    let frame = compositor().render(&p, 6, 1, &mut images);
    let reds: Vec<u8> = rows(&frame)[0].iter().map(|px| px[0]).collect();
    assert_eq!(reds, vec![191, 255, 255, 255, 64, 0]);
    // Flat colours (slates, placeholders) get the same edges.
    let p = plan(
        6,
        1,
        vec![layer(
            LayerSource::Slate { asset: media(3) },
            (4, 1),
            translate(0.25, 0.0),
        )],
    );
    let frame = compositor().render(&p, 6, 1, &mut NoImages);
    let reds: Vec<u8> = rows(&frame)[0].iter().map(|px| px[0]).collect();
    let s = u32::from(SLATE[0]);
    let part = |cov: u32| ((s * cov + 127) / 255) as u8;
    assert_eq!(
        reds,
        vec![part(191), SLATE[0], SLATE[0], SLATE[0], part(64), 0]
    );
}

#[test]
fn downscaling_by_three_does_not_alias() {
    // 1 px black and white columns, drawn at a third of their size: every
    // output pixel should be a grey mix, not a sampled column.
    let (w, h) = (300u32, 30u32);
    let mut rgba = Vec::new();
    for _ in 0..h {
        for x in 0..w {
            let v = if x % 2 == 0 { 255 } else { 0 };
            rgba.extend_from_slice(&[v, v, v, 255]);
        }
    }
    let mut images = Images::default().with(1, Picture::new(w, h, rgba));
    let p = plan(w, h, vec![layer(still(1), (w, h), DMat3::IDENTITY)]);
    let frame = compositor().render(&p, 100, 10, &mut images);
    let row: Vec<u8> = rows(&frame)[5].iter().map(|px| px[0]).collect();
    assert!(row.iter().all(|v| (40..=215).contains(v)), "{row:?}");
    // Rendering is still deterministic with mip levels in play.
    let again = compositor()
        .with_threads(3)
        .render(&p, 100, 10, &mut images);
    assert!(again == frame);
}

#[test]
fn centre_anchored_text_is_copied_without_resampling() {
    // A centred box is almost never a whole number of pixels: the raster
    // must still land on whole output pixels (TX-004: crisp text).
    let spec = TextSpec {
        size_px: 40.0,
        ..TextSpec::new("Hello caption text")
    };
    let mut comp = compositor();
    let raster = comp.text_renderer().rasterize(&spec, 1.0);
    let alphas = |rgba: &[u8], channel: usize| -> (usize, usize) {
        let full = rgba.chunks(4).filter(|p| p[channel] == 255).count();
        let partial = rgba
            .chunks(4)
            .filter(|p| p[channel] > 0 && p[channel] < 255)
            .count();
        (full, partial)
    };
    let want = alphas(&raster.image.rgba, 3);
    let size = raster.layout.size;
    assert!(
        size.0.fract() != 0.0 || size.1.fract() != 0.0,
        "pick a text whose box isn't whole pixels: {size:?}"
    );
    let p = plan(
        600,
        200,
        vec![text_layer((600, 200), translate(300.0, 100.0))],
    );
    let mut images = Images::default();
    images.texts.insert(
        clip(1),
        TextLayer {
            spec,
            layer_to_canvas: translate(300.0, 100.0),
            anchor: TextAnchor::Center,
            motion: None,
        },
    );
    let frame = comp.render(&p, 600, 200, &mut images);
    // White text over black: the red channel is the coverage.
    assert_eq!(alphas(&frame.rgba, 0), want);
    // A preview size whose y scale differs slightly from x (fit_size
    // rounding) still copies the raster.
    let frame = comp.render(&p, 600, 199, &mut images);
    assert_eq!(alphas(&frame.rgba, 0), want);
}

/// A busy plan: scaled, rotated, translucent, bilinear.
fn busy_plan(width: u32, height: u32) -> (freecut_timeline::plan::FramePlan, Images) {
    let images = Images::default()
        .with(1, pattern(640, 360))
        .with(2, pattern(97, 61))
        .with(3, solid(10, 10, [30, 200, 90, 180]));
    let rotated = translate(f64::from(width) / 2.0, f64::from(height) / 3.0)
        * DMat3::from_angle(0.5)
        * DMat3::from_scale(DVec2::splat(1.7));
    let mut faded = layer(still(3), (10, 10), DMat3::from_scale(DVec2::splat(9.5)));
    faded.opacity = 0.6;
    let p = plan(
        width,
        height,
        vec![
            layer(
                still(1),
                (640, 360),
                fit_transform((640, 360), (width, height)),
            ),
            layer(still(2), (97, 61), rotated),
            faded,
        ],
    );
    (p, images)
}

#[test]
fn rendering_is_deterministic_across_runs_and_thread_counts() {
    let (p, mut images) = busy_plan(320, 240);
    let one = Compositor::new(FontRegistry::bundled())
        .with_threads(1)
        .render(&p, 320, 240, &mut images);
    let again = Compositor::new(FontRegistry::bundled())
        .with_threads(1)
        .render(&p, 320, 240, &mut images);
    let many = Compositor::new(FontRegistry::bundled())
        .with_threads(4)
        .render(&p, 320, 240, &mut images);
    assert!(one == again, "two runs differ");
    assert!(one == many, "thread count changes the output");
    // render_into reuses a buffer of another size.
    let mut reused = Frame::new(7, 3);
    Compositor::new(FontRegistry::bundled()).render_into(&p, &mut reused, &mut images);
    assert_eq!((reused.width, reused.height), (7, 3));
    reused.width = 320;
    reused.height = 240;
    Compositor::new(FontRegistry::bundled()).render_into(&p, &mut reused, &mut images);
    assert!(reused == one);
}

/// Run with `cargo test -p freecut-render --release -- --ignored`.
#[test]
#[ignore = "timing; meaningful only in release builds"]
fn a_1080p_frame_with_three_layers_renders_fast() {
    let (p, mut images) = busy_plan(1920, 1080);
    for threads in [1, 8] {
        let mut comp = compositor().with_threads(threads);
        comp.render(&p, 1920, 1080, &mut images);
        let runs = 10;
        let start = Instant::now();
        for _ in 0..runs {
            comp.render(&p, 1920, 1080, &mut images);
        }
        let per_frame = start.elapsed() / runs;
        println!("1080p, 3 layers, {threads} thread(s): {per_frame:?} per frame");
        assert!(per_frame.as_millis() < 30, "{per_frame:?}");
    }
}
