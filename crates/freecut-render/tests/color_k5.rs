//! Auto color and colour match on the CPU path (CL-019, CL-020,
//! color-and-filters.md › CL-001 › *K5 as built*): the statistics correct
//! synthetic casts, refuse uniform frames and repeat exactly; Strength
//! scales the effective sliders; a self-match is the identity; gains are
//! limited; the lattice follows the reference with a match; and Auto and
//! Match export exactly as previewed.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::*;
use freecut_render::color::stats::{
    ColorStatsError, analysis_picture, auto_color, frame_stats, match_stats, ycc_stats,
};
use freecut_render::color::{
    BakeSize, ColorDigest, CompiledStack, Evaluator, NoLuts, bake_eval, grade, stack_input,
};
use freecut_render::cpu::{Compositor, Picture};
use freecut_render::text::FontRegistry;
use freecut_timeline::ids::ClipId;
use freecut_timeline::model::{
    AutoCorrection, AutoOffsets, AutoSource, AutoStyle, ColorParamId, ColorStack, MatchRef,
    MatchTransform, Section,
};
use freecut_timeline::time::Tick;

fn unit(v: [u8; 3]) -> [f32; 3] {
    v.map(|x| f32::from(x) / 255.0)
}

/// FreeCut's test chart (as in `tests/color_k3.rs`): a hue sweep, a grey
/// ramp and skin, foliage, sky and yellow patches.
fn chart() -> Picture {
    let (w, h) = (96u32, 48u32);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let px = if y < 16 {
                let t = x as f32 / w as f32 * 6.0;
                let f = t.fract();
                let c = match t as u32 {
                    0 => [1.0, f, 0.0],
                    1 => [1.0 - f, 1.0, 0.0],
                    2 => [0.0, 1.0, f],
                    3 => [0.0, 1.0 - f, 1.0],
                    4 => [f, 0.0, 1.0],
                    _ => [1.0, 0.0, 1.0 - f],
                };
                let light = 0.4 + 0.6 * (1.0 - (y as f32 / 16.0));
                c.map(|v| v * light)
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

/// `p` with each channel scaled (a cast) and the whole range squeezed into
/// `lo..hi` (a flat, dim exposure).
fn cast(p: &Picture, gains: [f32; 3], lo: f32, hi: f32) -> Picture {
    let mut out = p.clone();
    for px in out.rgba.chunks_mut(4) {
        for c in 0..3 {
            let v = f32::from(px[c]) / 255.0;
            let v = lo + (hi - lo) * (v * gains[c]).min(1.0);
            px[c] = (v * 255.0).round() as u8;
        }
    }
    out
}

/// The mean of each channel in linear light over the grey ramp rows (where
/// a neutral picture has R = G = B).
fn ramp_balance(p: &Picture) -> [f64; 3] {
    let mut sum = [0.0f64; 3];
    let mut n = 0.0;
    for y in 16..32 {
        for x in 8..88 {
            let i = ((y * p.width + x) * 4) as usize;
            for (c, total) in sum.iter_mut().enumerate() {
                *total += (f64::from(p.rgba[i + c]) / 255.0).powf(2.4);
            }
            n += 1.0;
        }
    }
    sum.map(|s| s / n)
}

fn corrected(p: &Picture, offsets: AutoOffsets) -> Picture {
    let s = ColorStack::default().with_auto(Some(AutoCorrection::new(
        offsets,
        AutoStyle::Natural,
        AutoSource::WholeClip,
    )));
    grade(&CompiledStack::new(&s), p, 2)
}

#[test]
fn synthetic_casts_move_toward_neutral() {
    let chart = chart();
    // A blue cast: Temperature warms it.
    let blue = cast(&chart, [0.88, 0.96, 1.0], 0.0, 1.0);
    let o = auto_color(&[frame_stats(&blue)], AutoStyle::Natural).unwrap();
    assert!(o.temperature > 10, "{o:?}");
    let before = ramp_balance(&blue);
    let after = ramp_balance(&corrected(&blue, o));
    let spread = |m: [f64; 3]| (m[2] / m[0]).ln().abs();
    assert!(
        spread(after) < spread(before) / 3.0,
        "{before:?} → {after:?}"
    );
    // A green cast: Tint toward magenta.
    let green = cast(&chart, [0.9, 1.0, 0.9], 0.0, 1.0);
    let o = auto_color(&[frame_stats(&green)], AutoStyle::Natural).unwrap();
    assert!(o.tint > 10, "{o:?}");
    let after = ramp_balance(&corrected(&green, o));
    assert!((after[1] / after[0]).ln().abs() < 0.05, "{after:?}");
    // A flat, dim picture: brighter and stretched toward 0.02 / 0.98.
    let dim = cast(&chart, [1.0; 3], 0.15, 0.55);
    let o = auto_color(&[frame_stats(&dim)], AutoStyle::Natural).unwrap();
    assert!(o.exposure > 0 && o.blacks < 0, "{o:?}");
    let fixed = frame_stats(&corrected(&dim, o));
    let again = auto_color(&[fixed], AutoStyle::Natural).unwrap();
    assert!(again.exposure.abs() <= 3, "{again:?}");
    // A full-range neutral chart needs nearly nothing.
    let o = auto_color(&[frame_stats(&chart)], AutoStyle::Natural).unwrap();
    assert!(o.largest() <= 12, "{o:?}");
    // Vivid adds Vibrance and Saturation on top.
    let v = auto_color(&[frame_stats(&chart)], AutoStyle::Vivid).unwrap();
    assert_eq!((v.vibrance, v.saturation), (20, 5));
    assert_eq!(v.exposure, o.exposure);
}

#[test]
fn uniform_frames_have_nothing_to_correct() {
    for rgb in [[0, 0, 0], [200, 30, 40], [255, 255, 255]] {
        let p = solid(32, 16, [rgb[0], rgb[1], rgb[2], 255]);
        assert_eq!(
            auto_color(&[frame_stats(&p)], AutoStyle::Natural),
            Err(ColorStatsError::NothingToCorrect),
            "{rgb:?}"
        );
    }
    let clear = solid(8, 8, [90, 90, 90, 0]);
    assert_eq!(
        auto_color(&[frame_stats(&clear)], AutoStyle::Natural),
        Err(ColorStatsError::NoPixels)
    );
    assert_eq!(
        auto_color(&[], AutoStyle::Natural),
        Err(ColorStatsError::NoPixels)
    );
}

#[test]
fn reruns_and_threads_give_identical_results() {
    let frames: Vec<Picture> = (0..5)
        .map(|i| cast(&chart(), [0.9, 1.0, 1.0 - 0.02 * i as f32], 0.05, 0.9))
        .collect();
    let run = || {
        let stats: Vec<_> = frames.iter().map(frame_stats).collect();
        auto_color(&stats, AutoStyle::Vivid).unwrap()
    };
    let first = run();
    assert_eq!(first, run());
    // The same analysis on other threads gives the same bits.
    let others: Vec<AutoOffsets> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..4).map(|_| s.spawn(run)).collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert!(others.iter().all(|o| *o == first));
    let a = ycc_stats(&frames[0]);
    let b = std::thread::spawn({
        let f = frames[0].clone();
        move || ycc_stats(&f)
    })
    .join()
    .unwrap();
    assert_eq!(a, b);
}

#[test]
fn strength_scales_the_effective_sliders() {
    let offsets = AutoOffsets {
        exposure: 30,
        temperature: -21,
        contrast: 10,
        ..AutoOffsets::default()
    };
    let auto = ColorStack::default()
        .with_auto(Some(AutoCorrection::new(
            offsets,
            AutoStyle::Natural,
            AutoSource::WholeClip,
        )))
        .with(ColorParamId::AutoStrength, 50);
    let plain = ColorStack::default()
        .with(ColorParamId::Exposure, 15)
        .with(ColorParamId::Temperature, -11)
        .with(ColorParamId::Contrast, 5);
    let a = CompiledStack::new(&auto);
    let b = CompiledStack::new(&plain);
    assert_eq!(a.digest, b.digest, "the renderer sees the same sliders");
    let p = chart();
    assert_eq!(grade(&a, &p, 2), grade(&b, &p, 2));
    // Auto off grades nothing; the section switch is in the digest.
    let off = ColorStack {
        sections_enabled: auto.sections_enabled.with(Section::Auto, false),
        ..auto.clone()
    };
    assert!(off.is_identity());
}

fn reference() -> MatchRef {
    MatchRef::Clip {
        clip: ClipId::from_raw(core::num::NonZeroU64::new(7).unwrap()),
        at: Tick::ZERO,
        graded: true,
    }
}

fn transform(target: &Picture, refer: &Picture) -> MatchTransform {
    let gains = match_stats(&[ycc_stats(target)], &ycc_stats(refer)).unwrap();
    let (gain, offset) = gains.to_fixed();
    MatchTransform::new(gain, offset, reference())
}

#[test]
fn matching_moves_the_statistics_onto_the_reference() {
    let refer = chart();
    let target = cast(&refer, [1.0, 0.9, 0.7], 0.1, 0.7);
    // A self-match is exactly the identity.
    let me = match_stats(&[ycc_stats(&target)], &ycc_stats(&target)).unwrap();
    assert_eq!(me.gain, [1.0; 3]);
    assert_eq!(me.offset, [0.0; 3]);
    assert!(!me.limited);
    assert!(transform(&target, &target).is_identity());
    let s = ColorStack::default().with_match(Some(transform(&target, &target)));
    assert!(s.is_identity() && !s.section_is_default(Section::Match));

    let xf = transform(&target, &refer);
    assert!(!xf.is_identity());
    let s = ColorStack::default().with_match(Some(xf.clone()));
    let moved = ycc_stats(&stack_input(&s, &target));
    let (want, got) = (ycc_stats(&refer), moved);
    let before = ycc_stats(&target).mean();
    for (c, before) in before.into_iter().enumerate() {
        let err = (got.mean()[c] - want.mean()[c]).abs();
        let was = (before - want.mean()[c]).abs();
        assert!(err < 0.02 && err <= was, "channel {c}: {err} (was {was})");
    }
    assert!((got.std()[0] - want.std()[0]).abs() < 0.03);
    // The grade agrees with the stack input when nothing else is on.
    let graded = grade(&CompiledStack::new(&s), &target, 2);
    let input = stack_input(&s, &target);
    let worst = graded
        .rgba
        .iter()
        .zip(&input.rgba)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap();
    assert!(worst <= 2, "{worst}/255");
    // Intensity 0 changes nothing; the stack input follows the switch.
    let faint = s.with(ColorParamId::MatchIntensity, 0);
    assert!(faint.is_identity());
    assert_eq!(stack_input(&faint, &target), target);
}

#[test]
fn gains_are_limited_and_flat_frames_refused() {
    let day = chart();
    let night = cast(&day, [1.0; 3], 0.02, 0.12);
    let g = match_stats(&[ycc_stats(&night)], &ycc_stats(&day)).unwrap();
    assert!(g.limited);
    assert_eq!(g.gain[0], 2.0, "Y' gain held at 2×");
    // A strongly tinted reference: the Cb/Cr means move by at most 0.1.
    let tinted = cast(&day, [1.0, 0.3, 0.1], 0.0, 1.0);
    let g = match_stats(&[ycc_stats(&day)], &ycc_stats(&tinted)).unwrap();
    let t = ycc_stats(&day).mean();
    for (c, t) in t.into_iter().enumerate().skip(1) {
        let shift = (t * g.gain[c] + g.offset[c]) - t;
        assert!(shift.abs() <= 0.1 + 1e-9, "channel {c}: {shift}");
    }
    assert!(g.gain.iter().all(|v| (0.5..=2.0).contains(v)));
    let flat = solid(16, 16, [40, 40, 40, 255]);
    assert_eq!(
        match_stats(&[ycc_stats(&flat)], &ycc_stats(&day)),
        Err(ColorStatsError::NotEnoughDetail)
    );
    assert_eq!(
        match_stats(&[ycc_stats(&day)], &ycc_stats(&flat)),
        Err(ColorStatsError::NotEnoughDetail)
    );
    assert_eq!(
        match_stats(&[], &ycc_stats(&day)),
        Err(ColorStatsError::NoPixels)
    );
}

#[test]
fn analysis_pictures_are_box_filtered_to_512() {
    let big = Picture::new(2048, 8, [10u8, 20, 30, 255].repeat(2048 * 8));
    let small = analysis_picture(&big);
    assert_eq!((small.width, small.height), (512, 2));
    assert_eq!(frame_stats(&small).count, 1024);
    assert_eq!(&small.rgba[..4], &[10, 20, 30, 255]);
}

/// Max and mean error of the lattice against the reference, in 1/255, on
/// 32³ inputs between the nodes.
fn lattice_error(s: &ColorStack) -> (f32, f64, BakeSize) {
    let eval = Evaluator::with_luts(s, &NoLuts);
    let size = BakeSize::for_evaluator(&eval);
    let lattice = bake_eval(&eval, size, 4);
    let (mut max, mut sum, mut n) = (0.0f32, 0.0f64, 0u32);
    for r in 0..32u8 {
        for g in 0..32u8 {
            for b in 0..32u8 {
                let px = [3 + 8 * r, 3 + 8 * g, 3 + 8 * b];
                let want = eval.eval(unit(px)).map(|x| x.clamp(0.0, 1.0));
                let got = lattice.lookup(px).map(|x| x.clamp(0.0, 1.0));
                for c in 0..3 {
                    let e = (want[c] - got[c]).abs() * 255.0;
                    max = max.max(e);
                    sum += f64::from(e);
                    n += 1;
                }
            }
        }
    }
    (max, sum / f64::from(n), size)
}

fn xf(gain: [i32; 3], offset: [i32; 3]) -> ColorStack {
    ColorStack::default().with_match(Some(MatchTransform::new(gain, offset, reference())))
}

#[test]
fn the_lattice_follows_the_reference_with_a_match() {
    let cases = [
        ("brighter", xf([13_000, 10_000, 10_000], [-500, 0, 0])),
        ("darker, flatter", xf([6_000, 9_000, 9_000], [2_000, 0, 0])),
        ("warm shift", xf([10_000, 10_000, 10_000], [0, 300, 600])),
        (
            "cool, saturated",
            xf([10_000, 18_000, 18_000], [0, -400, -900]),
        ),
        (
            "extreme",
            xf([20_000, 20_000, 5_000], [-5_000, 1_000, -1_000]),
        ),
        (
            "match 60 % with sliders",
            xf([12_000, 11_000, 9_000], [-300, 200, -150])
                .with(ColorParamId::MatchIntensity, 60)
                .with(ColorParamId::Exposure, 15)
                .with(ColorParamId::Saturation, 20),
        ),
    ];
    for (name, s) in cases {
        let (max, mean, size) = lattice_error(&s);
        println!(
            "{name}: max {max:.2}/255, mean {mean:.3}/255 at {}³",
            size.nodes()
        );
        assert!(max <= 2.0 && mean <= 0.4, "{name}: max {max}, mean {mean}");
    }
}

#[test]
fn auto_and_match_grade_alike_on_any_thread_count() {
    let s = xf([12_000, 11_000, 9_000], [-300, 200, -150]).with_auto(Some(AutoCorrection::new(
        AutoOffsets {
            exposure: 12,
            temperature: -8,
            blacks: -4,
            ..AutoOffsets::default()
        },
        AutoStyle::Vivid,
        AutoSource::WholeClip,
    )));
    let p = chart();
    let c = CompiledStack::new(&s);
    let one = grade(&c, &p, 1);
    for threads in [2, 3, 8] {
        assert_eq!(grade(&c, &p, threads), one, "{threads} threads");
    }
    // The digest names what grades: a different match is a new digest.
    let other = xf([12_000, 11_000, 9_001], [-300, 200, -150]);
    assert_ne!(
        ColorDigest::of(&s, BakeSize::N33),
        ColorDigest::of(&other, BakeSize::N33)
    );
}

/// Checked-in PNGs of the chart through an auto correction of a blue cast
/// and through a match to a warm reference. `FREECUT_BLESS_GOLDEN=1`
/// rewrites them.
#[test]
fn k5_grades_match_their_golden_pictures() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/color");
    let bless = std::env::var_os("FREECUT_BLESS_GOLDEN").is_some();
    let base = chart();
    let blue = cast(&base, [0.88, 0.96, 1.0], 0.05, 0.85);
    let auto = auto_color(&[frame_stats(&blue)], AutoStyle::Natural).unwrap();
    let warm = cast(&base, [1.0, 0.85, 0.6], 0.0, 1.0);
    let cases = [
        (
            "auto_blue_cast",
            blue.clone(),
            ColorStack::default().with_auto(Some(AutoCorrection::new(
                auto,
                AutoStyle::Natural,
                AutoSource::WholeClip,
            ))),
        ),
        (
            "match_to_warm",
            base.clone(),
            ColorStack::default().with_match(Some(transform(&base, &warm))),
        ),
    ];
    for (name, source, s) in cases {
        let graded = grade(&CompiledStack::new(&s), &source, 2);
        let path = dir.join(format!("{name}.png"));
        if bless {
            image::RgbaImage::from_raw(graded.width, graded.height, graded.rgba.clone())
                .unwrap()
                .save(&path)
                .unwrap();
        }
        let want = image::open(&path)
            .unwrap_or_else(|e| {
                panic!("{}: {e}; bless with FREECUT_BLESS_GOLDEN=1", path.display())
            })
            .to_rgba8();
        assert_eq!(want.as_raw(), &graded.rgba, "{name}");
    }
}

/// Auto with a keyed Strength and a match with a keyed Intensity on one
/// still, rendered by the preview's compositor and by the exporter: the
/// same bytes on every frame (the acceptance of K5).
#[test]
fn auto_and_match_export_exactly_as_previewed() {
    use freecut_render::export::ExportRenderer;
    use freecut_timeline::edit::{ColorEdit, EditRequest, InsertReq, InsertTarget, NewClip};
    use freecut_timeline::export::ExportClock;
    use freecut_timeline::model::{MediaInfo, PropPath, PropValue};
    use freecut_timeline::time::{FrameRate, TimeRange};

    let f = |n: i64| Tick::from_raw(n * 705_600_000 / 30);
    let mut p = common::project::Project::new(160, 90, 30);
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
    let source = cast(&chart(), [0.88, 0.96, 1.0], 0.05, 0.85);
    let offsets = auto_color(&[frame_stats(&source)], AutoStyle::Natural).unwrap();
    for edit in [
        ColorEdit::SetAuto(Some(AutoCorrection::new(
            offsets,
            AutoStyle::Natural,
            AutoSource::WholeClip,
        ))),
        ColorEdit::SetMatch(Some(MatchTransform::new(
            [11_000, 12_000, 12_000],
            [-200, 150, 300],
            MatchRef::Clip {
                clip,
                at: Tick::ZERO,
                graded: true,
            },
        ))),
        ColorEdit::SetParam(ColorParamId::Exposure, 8),
    ] {
        p.apply(EditRequest::EditColor {
            seq,
            clips: vec![clip],
            edit,
        });
    }
    let strength = PropPath::Color(ColorParamId::AutoStrength);
    p.key(clip, strength, f(0), PropValue::Level(0));
    p.key(clip, strength, f(29), PropValue::Level(100));
    let intensity = PropPath::Color(ColorParamId::MatchIntensity);
    p.key(clip, intensity, f(0), PropValue::Level(100));
    p.key(clip, intensity, f(29), PropValue::Level(30));

    let images = || Images::default().with(still.get(), source.clone());
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
        Compositor::new(FontRegistry::bundled())
            .with_threads(3)
            .with_luts(Arc::new(NoLuts)),
        images(),
    )
    .unwrap();
    let mut preview = Compositor::new(FontRegistry::bundled()).with_threads(1);
    let mut preview_images = images();
    let mut frames = Vec::new();
    for k in [0u64, 15, 29] {
        let exported = export.render_frame(k).unwrap().clone();
        let plan = p.plan_at(clock.frame_time(k).unwrap());
        let previewed = preview.render(&plan, 160, 90, &mut preview_images);
        assert_eq!(exported, previewed, "frame {k}");
        frames.push(exported);
    }
    assert_ne!(frames[0], frames[2], "the keyed Strength moves the grade");
    assert_ne!(frames[0], frames[1]);
}
