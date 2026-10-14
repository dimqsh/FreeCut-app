//! The colour stack on the CPU path (CL-001, CL-007, CL-008, CL-035,
//! README C52, render-compositor.md §17): the lattice against the
//! reference evaluator, the properties the spec names, determinism across
//! thread counts, preview versus export, and golden pictures per slider
//! extreme.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::*;
use freecut_render::color::{
    BakeSize, ColorDigest, CompiledStack, Evaluator, bake_with, eval_pixel, grade, luma, quantize,
};
use freecut_render::cpu::{Compositor, Picture};
use freecut_render::text::FontRegistry;
use freecut_timeline::glam::DMat3;
use freecut_timeline::model::{ColorParamId, ColorStack, Section};
use proptest::prelude::*;

fn stack(values: &[(ColorParamId, i16)]) -> ColorStack {
    values
        .iter()
        .fold(ColorStack::default(), |s, (id, v)| s.with(*id, *v))
}

/// Every slider alone at both ends, every pair of sliders at +100 and at
/// −100, and some everyday mixes. (Everything at ±100 at once is checked
/// for finite values only; [`extreme_stacks_are_reported`] prints its
/// error.)
fn corner_stacks() -> Vec<(String, ColorStack)> {
    let mut out = Vec::new();
    for id in ColorParamId::LIGHT.into_iter().chain(ColorParamId::COLOR) {
        for v in [-100, 100] {
            let mut values = vec![(id, v)];
            if id == ColorParamId::ContrastPivot {
                values.push((ColorParamId::Contrast, 100));
            }
            out.push((format!("{id:?} {v}"), stack(&values)));
        }
    }
    let sliders: Vec<ColorParamId> = ColorParamId::LIGHT
        .into_iter()
        .chain(ColorParamId::COLOR)
        .collect();
    for (i, a) in sliders.iter().enumerate() {
        for b in &sliders[i + 1..] {
            for v in [-100, 100] {
                out.push((format!("{a:?} {v} + {b:?} {v}"), stack(&[(*a, v), (*b, v)])));
            }
        }
    }
    out.push((
        "warm, bright, punchy at 60 %".into(),
        stack(&[
            (ColorParamId::Temperature, 40),
            (ColorParamId::Exposure, 20),
            (ColorParamId::Contrast, 30),
            (ColorParamId::Vibrance, 50),
            (ColorParamId::Amount, 60),
        ]),
    ));
    out.push((
        "teal shift, crushed".into(),
        stack(&[
            (ColorParamId::Hue, -35),
            (ColorParamId::Blacks, -60),
            (ColorParamId::Saturation, 70),
            (ColorParamId::Highlights, -50),
        ]),
    ));
    out
}

/// 32 values per channel between the lattice's nodes (3, 11, …, 251).
fn samples() -> impl Iterator<Item = [u8; 3]> {
    let values: Vec<u8> = (0..32).map(|i| 3 + 8 * i as u8).collect();
    let v = values.clone();
    values.into_iter().flat_map(move |r| {
        let v = v.clone();
        v.clone()
            .into_iter()
            .flat_map(move |g| v.clone().into_iter().map(move |b| [r, g, b]))
    })
}

fn unit(v: [u8; 3]) -> [f32; 3] {
    v.map(|x| f32::from(x) / 255.0)
}

#[test]
fn the_lattice_follows_the_reference_within_the_bound() {
    let mut worst = Vec::new();
    for (name, s) in corner_stacks() {
        let lattice = bake_with(&s, BakeSize::N33, 4);
        let eval = Evaluator::new(&s);
        let (mut max, mut sum, mut count) = (0.0f32, 0.0f64, 0u32);
        for px in samples() {
            let want = eval.eval(unit(px)).map(|x| x.clamp(0.0, 1.0));
            let got = lattice.lookup(px).map(|x| x.clamp(0.0, 1.0));
            for i in 0..3 {
                let e = (want[i] - got[i]).abs() * 255.0;
                max = max.max(e);
                sum += f64::from(e);
                count += 1;
            }
        }
        let mean = sum / f64::from(count);
        worst.push((name, max, mean));
    }
    let report: Vec<String> = worst
        .iter()
        .map(|(name, max, mean)| format!("{name}: max {max:.3}/255, mean {mean:.4}/255"))
        .collect();
    eprintln!("{}", report.join("\n"));
    for (name, max, mean) in worst {
        assert!(max <= 2.0, "{name}: max error {max}/255");
        assert!(mean <= 0.4, "{name}: mean error {mean}/255");
    }
}

/// Every slider at +100 (or −100) at once: far steeper than anything the
/// lattice is meant for. The error is printed, not bounded (README C52's
/// bound is an estimate per control and pair).
#[test]
fn extreme_stacks_are_reported() {
    let sliders: Vec<ColorParamId> = ColorParamId::LIGHT
        .into_iter()
        .chain(ColorParamId::COLOR)
        .collect();
    for v in [-100i16, 100] {
        let s = stack(&sliders.iter().map(|id| (*id, v)).collect::<Vec<_>>());
        let lattice = bake_with(&s, BakeSize::N33, 2);
        let eval = Evaluator::new(&s);
        let max = samples()
            .flat_map(|px| {
                let want = eval.eval(unit(px)).map(|x| x.clamp(0.0, 1.0));
                let got = lattice.lookup(px).map(|x| x.clamp(0.0, 1.0));
                (0..3).map(move |i| (want[i] - got[i]).abs() * 255.0)
            })
            .fold(0.0f32, f32::max);
        assert!(max.is_finite());
        eprintln!("every slider at {v}: lattice max error {max:.2}/255");
    }
}

#[test]
fn a_forced_identity_bake_reproduces_every_8_bit_input() {
    let lattice = bake_with(&ColorStack::default(), BakeSize::N33, 2);
    for r in 0..=255u8 {
        for g in (0..=255u8).step_by(5) {
            for b in (0..=255u8).step_by(3) {
                let out = lattice.lookup([r, g, b]).map(quantize);
                assert_eq!(out, [r, g, b]);
            }
        }
    }
}

#[test]
fn exposure_is_monotonic() {
    let grey = [0.4f32; 3];
    let mut last = f32::NEG_INFINITY;
    for e in (-100..=100).step_by(10) {
        let v = eval_pixel(&stack(&[(ColorParamId::Exposure, e)]), grey)[0];
        assert!(v > last, "exposure {e}: {v} after {last}");
        last = v;
    }
    // And every input brightens with it.
    let up = Evaluator::new(&stack(&[(ColorParamId::Exposure, 50)]));
    let mut prev = f32::NEG_INFINITY;
    for v in 0..=255u8 {
        let out = up.eval(unit([v, v, v]))[0];
        assert!(out >= prev);
        prev = out;
    }
}

#[test]
fn hue_plus_100_equals_minus_100_and_greys_never_turn() {
    let plus = Evaluator::new(&stack(&[(ColorParamId::Hue, 100)]));
    let minus = Evaluator::new(&stack(&[(ColorParamId::Hue, -100)]));
    for px in samples() {
        assert_eq!(plus.eval(unit(px)), minus.eval(unit(px)), "{px:?}");
    }
    let turned = Evaluator::new(&stack(&[(ColorParamId::Hue, 63)]));
    for v in (0..=255u8).step_by(5) {
        let out = turned.eval(unit([v, v, v])).map(quantize);
        assert_eq!(out, [v, v, v], "grey {v}");
    }
}

#[test]
fn saturation_minus_100_gives_grey() {
    let s = stack(&[(ColorParamId::Saturation, -100)]);
    let eval = Evaluator::new(&s);
    let lattice = bake_with(&s, BakeSize::N33, 2);
    for px in samples() {
        let out = eval.eval(unit(px));
        assert!(out[0] == out[1] && out[1] == out[2], "{px:?} → {out:?}");
        let q = lattice.lookup(px).map(quantize);
        assert!(
            q[0].abs_diff(q[1]) <= 1 && q[1].abs_diff(q[2]) <= 1,
            "{px:?} → {q:?}"
        );
    }
    // Picture bytes come from the lattice: greys stay grey and a graded
    // colour picture has R = G = B within one step.
    let compiled = CompiledStack::new(&s);
    let out = grade(&compiled, &pattern(64, 32), 2);
    for px in out.rgba.chunks(4) {
        assert!(px[0].abs_diff(px[1]) <= 1 && px[1].abs_diff(px[2]) <= 1);
    }
}

#[test]
fn white_balance_keeps_luma_within_2_percent_on_a_grey_ramp() {
    for (t, g) in [
        (100, 0),
        (-100, 0),
        (0, 100),
        (0, -100),
        (100, 100),
        (-100, -100),
        (100, -100),
    ] {
        let s = stack(&[(ColorParamId::Temperature, t), (ColorParamId::Tint, g)]);
        let eval = Evaluator::new(&s);
        for v in (16..=240u8).step_by(8) {
            let x = f32::from(v) / 255.0;
            let y = luma(eval.eval([x; 3]));
            assert!(
                (y - x).abs() <= 0.02 * x,
                "T {t} tint {g} grey {v}: luma {y}"
            );
        }
    }
    // Warm is warmer: red up, blue down.
    let warm = eval_pixel(&stack(&[(ColorParamId::Temperature, 60)]), [0.5; 3]);
    assert!(warm[0] > 0.5 && warm[2] < 0.5);
}

#[test]
fn switches_and_amount_shape_the_result() {
    let s = stack(&[(ColorParamId::Exposure, 40), (ColorParamId::Tint, 30)]);
    let px = [0.3, 0.5, 0.2];
    let off = ColorStack {
        sections_enabled: s
            .sections_enabled
            .with(Section::Light, false)
            .with(Section::Color, false),
        ..s.clone()
    };
    assert_eq!(eval_pixel(&off, px), px);
    let full = eval_pixel(&s, px);
    let half = eval_pixel(&s.with(ColorParamId::Amount, 50), px);
    for i in 0..3 {
        let mid = (px[i] + full[i]) / 2.0;
        assert!((half[i] - mid).abs() < 1e-5);
    }
    assert_eq!(eval_pixel(&s.with(ColorParamId::Amount, 0), px), px);
    // A switched-off section's values don't change the digest.
    let off_light = ColorStack {
        sections_enabled: s.sections_enabled.with(Section::Light, false),
        ..s.clone()
    };
    let other = off_light.with(ColorParamId::Exposure, -90);
    assert_eq!(
        ColorDigest::of(&off_light, BakeSize::N33),
        ColorDigest::of(&other, BakeSize::N33)
    );
    assert_ne!(
        ColorDigest::of(&s, BakeSize::N33),
        ColorDigest::of(&s.with(ColorParamId::Exposure, 41), BakeSize::N33)
    );
}

#[test]
fn grading_keeps_alpha_and_transparent_pixels() {
    let s = CompiledStack::new(&stack(&[(ColorParamId::Brightness, 70)]));
    let src = picture(&[&[[10, 20, 30, 0], [10, 20, 30, 128], [200, 100, 50, 255]]]);
    let out = grade(&s, &src, 1);
    assert_eq!(&out.rgba[0..4], &[10, 20, 30, 0], "untouched");
    assert_eq!(out.rgba[7], 128);
    assert_eq!(out.rgba[11], 255);
    assert!(out.rgba[4] > 10, "graded");
}

#[test]
fn grading_does_not_depend_on_the_thread_count() {
    let s = CompiledStack::new(&stack(&[
        (ColorParamId::Hue, 25),
        (ColorParamId::Shadows, 40),
        (ColorParamId::Vibrance, -30),
    ]));
    let src = pattern(301, 97);
    let one = grade(&s, &src, 1);
    for threads in [2, 3, 8] {
        assert_eq!(grade(&s, &src, threads), one, "{threads} threads");
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// Any mix of sliders at any values stays finite on any input.
    #[test]
    fn every_stack_stays_finite(
        values in proptest::collection::vec((0usize..15, -100i16..=100), 0..16),
        corners in proptest::collection::vec(prop::bool::ANY, 15),
        px in prop::array::uniform3(0u8..=255),
    ) {
        let mut s = ColorStack::default();
        for (i, v) in values {
            s = s.with(ColorParamId::ALL[i], v);
        }
        for (i, corner) in corners.into_iter().enumerate() {
            if corner {
                let spec = ColorParamId::ALL[i].spec();
                let v = if i % 2 == 0 { spec.max } else { spec.min };
                s = s.with(ColorParamId::ALL[i], v);
            }
        }
        let out = eval_pixel(&s, unit(px));
        prop_assert!(out.iter().all(|v| v.is_finite()), "{out:?}");
    }
}

fn layer_graded(stack: Option<ColorStack>) -> freecut_timeline::plan::LayerPlan {
    let mut l = layer(still(1), (64, 36), DMat3::IDENTITY);
    l.color = stack.map(Arc::new);
    l
}

#[test]
fn the_compositor_grades_stills_once_and_keeps_them_cached() {
    let s = stack(&[(ColorParamId::Temperature, 50)]);
    let plan = plan(64, 36, vec![layer_graded(Some(s.clone()))]);
    let mut images = Images::default().with(1, pattern(64, 36));
    let mut c = Compositor::new(FontRegistry::bundled()).with_threads(2);
    let first = c.render(&plan, 64, 36, &mut images);
    assert_eq!(c.cached_grades(), (1, 1));
    let again = c.render(&plan, 64, 36, &mut images);
    assert_eq!(first, again);
    assert_eq!(c.cached_grades(), (1, 1), "the paused frame graded once");
    // The grade is the lattice's: the same as grading the picture directly.
    let direct = grade(&CompiledStack::new(&s), &pattern(64, 36), 1);
    let plain = plan_plain();
    let ungraded = Compositor::new(FontRegistry::bundled()).render(
        &plain,
        64,
        36,
        &mut Images::default().with(1, direct.clone()),
    );
    assert_eq!(first, ungraded);
}

fn plan_plain() -> freecut_timeline::plan::FramePlan {
    plan(64, 36, vec![layer_graded(None)])
}

#[test]
fn one_and_eight_threads_give_identical_frames() {
    let s = stack(&[
        (ColorParamId::Contrast, 45),
        (ColorParamId::Saturation, 30),
        (ColorParamId::ToneLift, 20),
    ]);
    let plan = plan(
        320,
        180,
        vec![{
            let mut l = layer_graded(Some(s));
            l.display_size = (300, 170);
            l.layer_to_canvas = DMat3::from_translation((7.5, 3.25).into());
            l
        }],
    );
    let render = |threads| {
        let mut images = Images::default().with(1, pattern(300, 170));
        Compositor::new(FontRegistry::bundled())
            .with_threads(threads)
            .render(&plan, 320, 180, &mut images)
    };
    assert_eq!(render(1), render(8));
}

/// FreeCut's test chart: a hue sweep across, a grey ramp down the middle
/// band, skin-like and saturated patches along the bottom.
fn chart() -> Picture {
    let (w, h) = (96u32, 48u32);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let px = if y < 16 {
                let hue = x as f32 / w as f32 * 6.0;
                let f = hue.fract();
                let (r, g, b) = match hue as u32 {
                    0 => (1.0, f, 0.0),
                    1 => (1.0 - f, 1.0, 0.0),
                    2 => (0.0, 1.0, f),
                    3 => (0.0, 1.0 - f, 1.0),
                    4 => (f, 0.0, 1.0),
                    _ => (1.0, 0.0, 1.0 - f),
                };
                let light = 0.35 + 0.65 * (1.0 - (y as f32 / 16.0));
                [r * light, g * light, b * light]
            } else if y < 32 {
                let v = x as f32 / (w - 1) as f32;
                [v, v, v]
            } else {
                match x / 16 {
                    0 => [0.87, 0.67, 0.55],
                    1 => [0.65, 0.45, 0.35],
                    2 => [0.40, 0.26, 0.20],
                    3 => [0.20, 0.45, 0.20],
                    4 => [0.25, 0.45, 0.75],
                    _ => [0.90, 0.85, 0.20],
                }
            };
            rgba.extend(px.map(|v| (v * 255.0).round() as u8));
            rgba.push(255);
        }
    }
    Picture::new(w, h, rgba)
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/color")
}

fn png(picture: &Picture) -> image::RgbaImage {
    image::RgbaImage::from_raw(picture.width, picture.height, picture.rgba.clone()).unwrap()
}

/// Checked-in PNGs of the test chart graded by each control at its
/// extremes (the future GPU goldens). `FREECUT_BLESS_GOLDEN=1` rewrites
/// them.
#[test]
fn slider_extremes_match_their_golden_pictures() {
    let dir = golden_dir();
    let bless = std::env::var_os("FREECUT_BLESS_GOLDEN").is_some();
    if bless {
        std::fs::create_dir_all(&dir).unwrap();
    }
    let source = chart();
    let mut cases: Vec<(String, ColorStack)> = vec![("chart".into(), ColorStack::default())];
    for id in ColorParamId::LIGHT.into_iter().chain(ColorParamId::COLOR) {
        if id == ColorParamId::ContrastPivot {
            continue;
        }
        for v in [-100, 100] {
            let sign = if v < 0 { "minus" } else { "plus" };
            cases.push((format!("{}_{sign}", id.as_str()), stack(&[(id, v)])));
        }
    }
    for (name, s) in cases {
        let graded = grade(&CompiledStack::new(&s), &source, 2);
        let path = dir.join(format!("{name}.png"));
        if bless {
            png(&graded).save(&path).unwrap();
        }
        let want = image::open(&path)
            .unwrap_or_else(|e| {
                panic!("{}: {e}; bless with FREECUT_BLESS_GOLDEN=1", path.display())
            })
            .to_rgba8();
        assert_eq!(want.as_raw(), &graded.rgba, "{name}");
    }
}

/// The budget lines of README C52 (release, 8 threads, 1080p):
/// `cargo test --release -p freecut-render --test color color_bench -- --ignored --nocapture`.
#[test]
#[ignore = "benchmark: run in release"]
fn color_bench() {
    use std::time::Instant;
    let s = stack(&[
        (ColorParamId::Exposure, 20),
        (ColorParamId::Contrast, 30),
        (ColorParamId::Temperature, 25),
        (ColorParamId::Hue, 10),
        (ColorParamId::Vibrance, 40),
    ]);
    let time = |f: &mut dyn FnMut()| {
        f();
        let runs = 10;
        let t = Instant::now();
        for _ in 0..runs {
            f();
        }
        t.elapsed().as_secs_f64() * 1000.0 / f64::from(runs)
    };
    let frame = pattern(1920, 1080);
    for threads in [1, 8] {
        let bake_ms = time(&mut || {
            std::hint::black_box(bake_with(&s, BakeSize::N33, threads));
        });
        let compiled = CompiledStack::new(&s);
        let grade_ms = time(&mut || {
            std::hint::black_box(grade(&compiled, &frame, threads));
        });
        eprintln!("{threads} threads: bake 33³ {bake_ms:.2} ms, grade 1080p {grade_ms:.2} ms");
    }
}

/// A keyframed temperature ramp on a still, rendered by the preview's
/// compositor and by the exporter: the same bytes on every frame
/// (README C52's parity), and the grade really changes over time.
#[test]
fn a_keyframed_grade_exports_exactly_as_previewed() {
    use common::project::Project;
    use freecut_render::export::ExportRenderer;
    use freecut_timeline::edit::{ColorEdit, EditRequest, InsertReq, InsertTarget, NewClip};
    use freecut_timeline::export::ExportClock;
    use freecut_timeline::model::{MediaInfo, PropPath, PropValue};
    use freecut_timeline::time::{FrameRate, Tick, TimeRange};

    let f = |n: i64| Tick::from_raw(n * 705_600_000 / 30);
    let mut p = Project::new(160, 90, 30);
    let still = p.import(
        "/pictures/chart.png",
        MediaInfo {
            container: "png_pipe".into(),
            default_video: None,
            duration: Tick::ZERO,
            video: Vec::new(),
            audio: Vec::new(),
            still: Some((96, 48)),
        },
    );
    let seq = p.seq();
    let main = p
        .editor
        .document()
        .sequence(seq)
        .unwrap()
        .main_track()
        .unwrap()
        .id;
    let clip = p.apply(EditRequest::Insert(InsertReq {
        seq,
        target: InsertTarget::Track(main),
        at: Tick::ZERO,
        clips: vec![NewClip::image(still, f(30))],
    }))[0];
    p.apply(EditRequest::EditColor {
        seq,
        clips: vec![clip],
        edit: ColorEdit::SetParams(vec![
            (ColorParamId::Contrast, 25),
            (ColorParamId::Saturation, 20),
        ]),
    });
    let temperature = PropPath::Color(ColorParamId::Temperature);
    p.key(clip, temperature, f(0), PropValue::Level(-80));
    p.key(clip, temperature, f(29), PropValue::Level(80));

    let images = || Images::default().with(still.get(), chart());
    let clock = ExportClock::new(
        TimeRange::new(Tick::ZERO, f(30)).unwrap(),
        FrameRate::new(30, 1).unwrap(),
    )
    .unwrap();
    let mut export = ExportRenderer::new(
        p.doc(),
        seq,
        clock,
        (160, 90),
        Compositor::new(FontRegistry::bundled()).with_threads(3),
        images(),
    )
    .unwrap();
    let mut preview = Compositor::new(FontRegistry::bundled()).with_threads(1);
    let mut preview_images = images();
    let mut frames = Vec::new();
    for k in [0u64, 7, 15, 22, 29] {
        let exported = export.render_frame(k).unwrap().clone();
        let plan = p.plan_at(clock.frame_time(k).unwrap());
        let previewed = preview.render(&plan, 160, 90, &mut preview_images);
        assert_eq!(exported, previewed, "frame {k}");
        frames.push(exported);
    }
    assert_ne!(frames[0], frames[4], "the ramp warms the picture");
    let mid = frames[0].pixel(80, 60).unwrap();
    let end = frames[4].pixel(80, 60).unwrap();
    assert!(end[0] > mid[0] && end[2] < mid[2], "{mid:?} → {end:?}");
}
