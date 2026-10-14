//! The user LUT and skin protection on the CPU path (CL-017, CL-030,
//! color-and-filters.md › CL-001 › *K3 as built*): identity tables are
//! exact with both interpolations, the LUT-only fast path agrees with the
//! bake, a missing LUT grades nothing and is reported, skin protection
//! keeps skin patches and leaves the rest, the lattice follows the
//! reference, and a LUT at 60 % with skin protection exports exactly as
//! previewed.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::*;
use freecut_media::lut::{Lut, Lut1d, Lut3d};
use freecut_render::color::{
    BakeSize, ColorDigest, CompiledStack, Evaluator, LutSet, NoLuts, bake_eval, eval_pixel_with,
    grade, skin_mask,
};
use freecut_render::cpu::{Compositor, Picture};
use freecut_render::text::FontRegistry;
use freecut_timeline::model::{
    ColorParamId, ColorStack, LutInterp, LutRef, LutSlot, Section, SkinProtect, SkinScope,
};

fn unit(v: [u8; 3]) -> [f32; 3] {
    v.map(|x| f32::from(x) / 255.0)
}

fn lut3(size: u32, f: impl FnMut([f32; 3]) -> [f32; 3]) -> Lut {
    Lut {
        title: None,
        shaper: None,
        cube: Some(Lut3d::from_fn(size, f).unwrap()),
        domain_min: [0.0; 3],
        domain_max: [1.0; 3],
        skipped_keywords: 0,
        notices: Vec::new(),
    }
}

/// A strong teal-and-orange look: shadows toward teal, highlights toward
/// orange, more saturation (FreeCut's own test table).
fn teal_orange(c: [f32; 3]) -> [f32; 3] {
    let y = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let s = 1.0 - y;
    [
        c[0] - 0.18 * s + 0.12 * y,
        c[1] + 0.04 * s,
        c[2] + 0.16 * s - 0.14 * y,
    ]
}

const TEAL: [u8; 16] = [0x7e; 16];
const IDENT: [u8; 16] = [0x1d; 16];

fn luts() -> LutSet {
    LutSet::new()
        .with(TEAL, lut3(17, teal_orange))
        .with(IDENT, lut3(33, |c| c))
}

fn with_lut(hash: [u8; 16], intensity: u8, interp: LutInterp, skin: Option<u8>) -> ColorStack {
    ColorStack::default().with_lut(Some(LutSlot {
        intensity,
        interp,
        skin: match skin {
            Some(amount) => SkinProtect { on: true, amount },
            None => SkinProtect::default(),
        },
        ..LutSlot::new(LutRef::new(hash, "test.cube"))
    }))
}

/// Every 8-bit colour at a stride, plus the ends.
fn every_input(step: usize) -> Picture {
    let mut rgba = Vec::new();
    let values: Vec<u8> = (0..=255).step_by(step).chain([255]).collect();
    for &r in &values {
        for &g in &values {
            for &b in &values {
                rgba.extend([r, g, b, 255]);
            }
        }
    }
    let n = values.len() as u32;
    Picture::new(n * n, n, rgba)
}

#[test]
fn identity_tables_are_exact_with_both_interpolations() {
    let set = LutSet::new()
        .with([2; 16], lut3(2, |c| c))
        .with([33; 16], lut3(33, |c| c));
    let source = every_input(5);
    for hash in [[2; 16], [33; 16]] {
        for interp in LutInterp::ALL {
            let s = with_lut(hash, 100, interp, None);
            let fast = CompiledStack::with_luts(&s, &set);
            assert!(fast.is_lut_only());
            assert_eq!(grade(&fast, &source, 3), source, "{interp:?} fast");
            let baked = CompiledStack::baked(&s, &set);
            assert!(!baked.is_lut_only());
            assert_eq!(grade(&baked, &source, 3), source, "{interp:?} baked");
        }
    }
}

#[test]
fn the_fast_path_agrees_with_the_bake() {
    let set = luts();
    let source = every_input(3);
    for interp in LutInterp::ALL {
        for intensity in [100, 60, 25] {
            let s = with_lut(TEAL, intensity, interp, None);
            let fast = grade(&CompiledStack::with_luts(&s, &set), &source, 2);
            let baked = grade(&CompiledStack::baked(&s, &set), &source, 2);
            let worst = fast
                .rgba
                .iter()
                .zip(&baked.rgba)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            assert!(worst <= 1, "{interp:?} {intensity}: {worst}/255");
        }
    }
    // Anything else in the stack, skin protection or Amount below 100
    // goes through the lattice.
    for s in [
        with_lut(TEAL, 100, LutInterp::Tetrahedral, Some(70)),
        with_lut(TEAL, 100, LutInterp::Tetrahedral, None).with(ColorParamId::Amount, 50),
        with_lut(TEAL, 100, LutInterp::Tetrahedral, None).with(ColorParamId::Exposure, 10),
    ] {
        assert!(!CompiledStack::with_luts(&s, &set).is_lut_only(), "{s:?}");
    }
}

#[test]
fn a_1d_lut_applies_per_channel_and_a_fine_one_bakes_at_65() {
    let gamma = Lut {
        title: None,
        shaper: Some(
            Lut1d::new(
                (0..1024)
                    .map(|i| [(i as f32 / 1023.0).powf(0.8); 3])
                    .collect(),
            )
            .unwrap(),
        ),
        cube: None,
        domain_min: [0.0; 3],
        domain_max: [1.0; 3],
        skipped_keywords: 0,
        notices: Vec::new(),
    };
    let set = LutSet::new()
        .with([9; 16], gamma)
        .with([65; 16], lut3(65, teal_orange));
    let s = with_lut([9; 16], 100, LutInterp::Tetrahedral, None);
    let out = eval_pixel_with(&s, &set, [0.25, 0.5, 0.75]);
    for (c, v) in [0.25f32, 0.5, 0.75].into_iter().enumerate() {
        assert!((out[c] - v.powf(0.8)).abs() < 1e-3, "{out:?}");
    }
    let fine = with_lut([65; 16], 80, LutInterp::Tetrahedral, None).with(ColorParamId::Hue, 5);
    assert_eq!(
        BakeSize::for_evaluator(&Evaluator::with_luts(&fine, &set)),
        BakeSize::N65
    );
    let coarse = with_lut(TEAL, 80, LutInterp::Tetrahedral, None).with(ColorParamId::Hue, 5);
    assert_eq!(
        BakeSize::for_evaluator(&Evaluator::with_luts(&coarse, &set)),
        BakeSize::N33
    );
}

#[test]
fn a_missing_lut_grades_nothing_and_is_reported() {
    let s = with_lut(TEAL, 100, LutInterp::Tetrahedral, None);
    let source = every_input(17);
    // Nothing else in the stack: the picture is untouched.
    let compiled = CompiledStack::with_luts(&s, &NoLuts);
    assert_eq!(grade(&compiled, &source, 1), source);
    // With a warm white balance too, only the LUT is left out.
    let warm = s.with(ColorParamId::Temperature, 40);
    let without = warm.with_lut(None);
    assert_eq!(
        grade(&CompiledStack::with_luts(&warm, &NoLuts), &source, 1),
        grade(&CompiledStack::new(&without), &source, 1)
    );
    // The digest tells a found LUT from a missing one.
    let found = CompiledStack::with_luts(&warm, &luts());
    let lost = CompiledStack::with_luts(&warm, &NoLuts);
    assert_ne!(found.digest, lost.digest);
    assert_eq!(
        lost.digest,
        ColorDigest::of(&without, BakeSize::N33),
        "a missing LUT digests as no LUT"
    );

    // The compositor reports it once per file and keeps drawing.
    let mut p = common::project::Project::new(64, 36, 30);
    let (seq, clip, still) = one_still(&mut p);
    for edit in [
        freecut_timeline::edit::ColorEdit::SetLut(Some(LutSlot::new(LutRef::new(
            TEAL,
            "gone.cube",
        )))),
        freecut_timeline::edit::ColorEdit::SetParam(ColorParamId::Temperature, 30),
    ] {
        p.apply(freecut_timeline::edit::EditRequest::EditColor {
            seq,
            clips: vec![clip],
            edit,
        });
    }
    let plan = p.plan_at(freecut_timeline::time::Tick::ZERO);
    let mut images = Images::default().with(still.get(), chart());
    let mut missing = Compositor::new(FontRegistry::bundled()).with_threads(2);
    let a = missing.render(&plan, 64, 36, &mut images);
    missing.render(&plan, 64, 36, &mut images);
    assert_eq!(
        missing.take_missing_luts(),
        vec![LutRef::new(TEAL, "gone.cube")]
    );
    assert_eq!(missing.take_missing_luts(), Vec::new(), "taken");
    let mut found = Compositor::new(FontRegistry::bundled()).with_luts(Arc::new(luts()));
    let b = found.render(&plan, 64, 36, &mut images);
    assert_ne!(a, b, "the found LUT grades");
    assert_eq!(found.take_missing_luts(), Vec::new());
}

#[test]
fn skin_protection_keeps_skin_and_grades_the_rest() {
    let set = luts();
    let skin = [unit([210, 160, 120]), unit([120, 78, 52])];
    let other = [
        unit([110, 160, 220]),
        unit([60, 120, 50]),
        unit([128, 128, 128]),
    ];
    for patch in skin {
        assert!(skin_mask(patch) > 0.99);
    }
    let off = with_lut(TEAL, 100, LutInterp::Tetrahedral, None);
    let on = with_lut(TEAL, 100, LutInterp::Tetrahedral, Some(100));
    let half = with_lut(TEAL, 100, LutInterp::Tetrahedral, Some(50));
    let dist = |a: [f32; 3], b: [f32; 3]| (0..3).map(|i| (a[i] - b[i]).abs()).fold(0.0, f32::max);
    for patch in skin {
        let graded = eval_pixel_with(&off, &set, patch);
        assert!(dist(graded, patch) > 0.025, "the LUT moves skin: {patch:?}");
        let kept = eval_pixel_with(&on, &set, patch);
        assert!(dist(kept, patch) < 1e-3, "Amount 100 keeps it: {kept:?}");
        let mid = eval_pixel_with(&half, &set, patch);
        for c in 0..3 {
            let want = patch[c] + 0.5 * (graded[c] - patch[c]);
            assert!((mid[c] - want).abs() < 2e-3);
        }
    }
    for patch in other {
        assert_eq!(
            eval_pixel_with(&on, &set, patch),
            eval_pixel_with(&off, &set, patch),
            "{patch:?} isn't skin"
        );
    }

    // The Color scope keeps white balance, saturation and HSL off skin but
    // not exposure.
    let cool = ColorStack::default()
        .with(ColorParamId::Temperature, -80)
        .with(ColorParamId::Saturation, -60);
    let cool_on = cool
        .with_skin(SkinScope::Color, true)
        .with(ColorParamId::SkinAmount(SkinScope::Color), 100);
    for patch in skin {
        assert!(dist(eval_pixel_with(&cool, &NoLuts, patch), patch) > 0.04);
        assert!(dist(eval_pixel_with(&cool_on, &NoLuts, patch), patch) < 1e-3);
        let brighter = cool_on.with(ColorParamId::Exposure, 30);
        let lifted = eval_pixel_with(&brighter, &NoLuts, patch);
        assert!(
            lifted.iter().zip(patch).all(|(a, b)| *a > b + 0.02),
            "{lifted:?}"
        );
    }
    for patch in other {
        assert_eq!(
            eval_pixel_with(&cool_on, &NoLuts, patch),
            eval_pixel_with(&cool, &NoLuts, patch)
        );
    }
    // A switched-off Color section takes its skin protection with it.
    let switched = ColorStack {
        sections_enabled: cool_on.sections_enabled.with(Section::Color, false),
        ..cool_on.clone()
    };
    assert_eq!(eval_pixel_with(&switched, &NoLuts, skin[0]), skin[0]);
}

/// Max and mean error of the lattice against the reference, in 1/255, on
/// 32³ inputs between the nodes.
fn lattice_error(s: &ColorStack, set: &LutSet) -> (f32, f64) {
    let eval = Evaluator::with_luts(s, set);
    let lattice = bake_eval(&eval, BakeSize::for_evaluator(&eval), 4);
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
    (max, sum / f64::from(n))
}

#[test]
fn the_lattice_follows_the_reference_with_luts_and_skin() {
    let set = luts();
    let cases = [
        (
            "LUT 60 %",
            with_lut(TEAL, 60, LutInterp::Tetrahedral, None).with(ColorParamId::Exposure, 20),
        ),
        (
            "trilinear",
            with_lut(TEAL, 100, LutInterp::Trilinear, None).with(ColorParamId::Contrast, 30),
        ),
        (
            "LUT skin",
            with_lut(TEAL, 100, LutInterp::Tetrahedral, Some(70)),
        ),
        (
            "Color skin",
            ColorStack::default()
                .with(ColorParamId::Temperature, -60)
                .with(ColorParamId::Saturation, 40)
                .with_skin(SkinScope::Color, true),
        ),
        (
            "both skins",
            with_lut(TEAL, 60, LutInterp::Tetrahedral, Some(70))
                .with(ColorParamId::Temperature, -40)
                .with(ColorParamId::Vibrance, 50)
                .with_skin(SkinScope::Color, true),
        ),
    ];
    for (name, s) in cases {
        let (max, mean) = lattice_error(&s, &set);
        println!("{name}: max {max:.2}/255, mean {mean:.3}/255");
        assert!(max <= 2.0 && mean <= 0.4, "{name}: max {max}, mean {mean}");
    }
}

/// FreeCut's test chart: a hue sweep, a grey ramp and skin, foliage, sky
/// and yellow patches along the bottom (as in `tests/color_k2.rs`).
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

/// Checked-in PNGs of the chart through a LUT, with and without skin
/// protection. `FREECUT_BLESS_GOLDEN=1` rewrites them.
#[test]
fn k3_looks_match_their_golden_pictures() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/color");
    let bless = std::env::var_os("FREECUT_BLESS_GOLDEN").is_some();
    let set = luts();
    let source = chart();
    for (name, s) in [
        (
            "lut_teal_orange_60",
            with_lut(TEAL, 60, LutInterp::Tetrahedral, None),
        ),
        (
            "lut_teal_orange_skin",
            with_lut(TEAL, 100, LutInterp::Tetrahedral, Some(100)),
        ),
        (
            "color_skin_cool",
            ColorStack::default()
                .with(ColorParamId::Temperature, -80)
                .with(ColorParamId::Saturation, -50)
                .with_skin(SkinScope::Color, true),
        ),
    ] {
        let graded = grade(&CompiledStack::with_luts(&s, &set), &source, 2);
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

/// A project with one still on the main track: its sequence, clip and
/// media.
fn one_still(
    p: &mut common::project::Project,
) -> (
    freecut_timeline::ids::SeqId,
    freecut_timeline::ids::ClipId,
    freecut_timeline::ids::MediaId,
) {
    use freecut_timeline::edit::{EditRequest, InsertReq, InsertTarget, NewClip};
    use freecut_timeline::model::MediaInfo;
    use freecut_timeline::time::Tick;
    let f = |n: i64| Tick::from_raw(n * 705_600_000 / 30);
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
    (seq, clip, still)
}

/// A LUT at 60 % with skin protection and a keyed Intensity, rendered by
/// the preview's compositor and by the exporter: the same bytes on every
/// frame (the acceptance of K3).
#[test]
fn a_lut_with_skin_protection_exports_exactly_as_previewed() {
    use freecut_render::export::ExportRenderer;
    use freecut_timeline::edit::{ColorEdit, EditRequest};
    use freecut_timeline::export::ExportClock;
    use freecut_timeline::model::{PropPath, PropValue};
    use freecut_timeline::time::{FrameRate, Tick, TimeRange};

    let f = |n: i64| Tick::from_raw(n * 705_600_000 / 30);
    let mut p = common::project::Project::new(160, 90, 30);
    let (seq, clip, still) = one_still(&mut p);
    for edit in [
        ColorEdit::SetLut(Some(LutSlot::new(LutRef::new(TEAL, "teal.cube")))),
        ColorEdit::SetParam(ColorParamId::LutIntensity, 60),
        ColorEdit::SetSkin {
            scope: SkinScope::Lut,
            on: true,
        },
    ] {
        p.apply(EditRequest::EditColor {
            seq,
            clips: vec![clip],
            edit,
        });
    }
    let intensity = PropPath::Color(ColorParamId::LutIntensity);
    p.key(clip, intensity, f(0), PropValue::Level(20));
    p.key(clip, intensity, f(29), PropValue::Level(60));

    let images = || Images::default().with(still.get(), chart());
    let clock = ExportClock::new(
        TimeRange::new(Tick::ZERO, f(30)).unwrap(),
        FrameRate::new(30, 1).unwrap(),
    )
    .unwrap();
    let shared: Arc<LutSet> = Arc::new(luts());
    let mut export = ExportRenderer::new(
        p.doc(),
        seq,
        clock,
        (160, 90),
        Compositor::new(FontRegistry::bundled())
            .with_threads(3)
            .with_luts(shared.clone()),
        images(),
    )
    .unwrap();
    let mut preview = Compositor::new(FontRegistry::bundled())
        .with_threads(1)
        .with_luts(shared);
    let mut preview_images = images();
    let mut frames = Vec::new();
    for k in [0u64, 15, 29] {
        let exported = export.render_frame(k).unwrap().clone();
        let plan = p.plan_at(clock.frame_time(k).unwrap());
        let previewed = preview.render(&plan, 160, 90, &mut preview_images);
        assert_eq!(exported, previewed, "frame {k}");
        frames.push(exported);
    }
    assert_ne!(frames[0], frames[2], "the keyed Intensity moves the grade");
    assert!(preview.take_missing_luts().is_empty());
}
