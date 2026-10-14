//! HSL, Curves and Wheels on the CPU path (CL-013, CL-014, CL-015,
//! color-and-filters.md › CL-001 › *K2 as built*): the lattice against the
//! reference evaluator (33³, and 65³ for steep curves), the properties the
//! specs name (band weights, greys, the hue wrap, luma-keeping pucks, the
//! corner values), the eyedropper's probe, golden pictures and preview
//! versus export.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::path::PathBuf;

use common::*;
use freecut_render::color::{
    BakeSize, ColorDigest, CompiledStack, Evaluator, bake_with, band_weights, eval_pixel, grade,
    hsl_band_at, luma, probe_hsl_input, puck_vector,
};
use freecut_render::cpu::{Compositor, Picture};
use freecut_render::text::FontRegistry;
use freecut_timeline::model::{
    ColorParamId, ColorStack, Curve, CurveChannel, CurvePoint, HslChannel, HueBand, Section,
    WheelId,
};
use proptest::prelude::*;

fn hsl(band: HueBand, channel: HslChannel) -> ColorParamId {
    ColorParamId::Hsl { band, channel }
}

fn curve(points: &[(u16, u16)]) -> Curve {
    Curve::new(
        points
            .iter()
            .map(|(x, y)| CurvePoint::new(*x, *y))
            .collect(),
    )
    .unwrap()
}

fn unit(v: [u8; 3]) -> [f32; 3] {
    v.map(|x| f32::from(x) / 255.0)
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

/// The S-curve of a typical grade.
fn s_curve() -> Curve {
    curve(&[(0, 0), (2_500, 1_800), (7_500, 8_200), (10_000, 10_000)])
}

/// Every HSL slider alone at both ends, every wheel's puck at the rim in
/// four directions, every luminance at both ends, Strength at half, and
/// curves from gentle to steep.
fn corner_stacks() -> Vec<(String, ColorStack)> {
    let mut out = Vec::new();
    for band in HueBand::ALL {
        for channel in HslChannel::ALL {
            for v in [-100, 100] {
                let s = ColorStack::default().with(hsl(band, channel), v);
                out.push((format!("{band:?} {channel:?} {v}"), s));
            }
        }
    }
    for wheel in WheelId::ALL {
        for (x, y) in [(1000, 0), (0, 1000), (-1000, 0), (0, -1000), (-707, 707)] {
            let s = ColorStack::default().with_puck(wheel, x, y);
            out.push((format!("{wheel:?} puck ({x}, {y})"), s));
        }
        for v in [-100, 100] {
            let s = ColorStack::default().with(ColorParamId::WheelLuma(wheel), v);
            out.push((format!("{wheel:?} luma {v}"), s));
        }
    }
    out.push((
        "lift-gamma-gain at half strength".into(),
        ColorStack::default()
            .with_puck(WheelId::Shadows, 600, -300)
            .with_puck(WheelId::Highlights, -500, 500)
            .with(ColorParamId::WheelLuma(WheelId::Midtones), 40)
            .with(ColorParamId::WheelStrength, 50),
    ));
    for (name, channel, c) in [
        ("S-curve", CurveChannel::Luma, s_curve()),
        (
            "inverted",
            CurveChannel::Luma,
            curve(&[(0, 10_000), (10_000, 0)]),
        ),
        (
            "levels",
            CurveChannel::Luma,
            curve(&[(1_500, 0), (8_500, 10_000)]),
        ),
        (
            "red lift",
            CurveChannel::Red,
            curve(&[(0, 0), (4_000, 5_500), (10_000, 10_000)]),
        ),
        (
            "blue crush",
            CurveChannel::Blue,
            curve(&[(0, 0), (3_000, 1_000), (10_000, 10_000)]),
        ),
        (
            "steep",
            CurveChannel::Green,
            curve(&[(0, 0), (4_500, 1_000), (5_500, 9_000), (10_000, 10_000)]),
        ),
        (
            "flat",
            CurveChannel::Luma,
            curve(&[(0, 5_000), (10_000, 5_000)]),
        ),
    ] {
        out.push((name.into(), ColorStack::default().with_curve(channel, c)));
    }
    out.push((
        "everyday mix".into(),
        ColorStack::default()
            .with(ColorParamId::Contrast, 20)
            .with_curve(CurveChannel::Luma, s_curve())
            .with_puck(WheelId::Shadows, 400, -200)
            .with(hsl(HueBand::Orange, HslChannel::Saturation), -30)
            .with(hsl(HueBand::Blue, HslChannel::Hue), 40)
            .with(ColorParamId::Saturation, 15),
    ));
    out
}

/// Max and mean error of the lattice `for_stack` picks, in 1/255.
fn lattice_error(s: &ColorStack) -> (f32, f64, BakeSize) {
    let size = BakeSize::for_stack(s);
    let lattice = bake_with(s, size, 4);
    let eval = Evaluator::new(s);
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
    (max, sum / f64::from(count), size)
}

#[test]
fn the_lattice_follows_the_reference_within_the_bound() {
    let mut rows = Vec::new();
    for (name, s) in corner_stacks() {
        let (max, mean, size) = lattice_error(&s);
        rows.push((name, max, mean, size));
    }
    for (name, max, mean, size) in &rows {
        eprintln!(
            "{name}: max {max:.3}/255, mean {mean:.4}/255 ({}³)",
            size.nodes()
        );
    }
    for (name, max, mean, _) in rows {
        assert!(max <= 2.0, "{name}: max error {max}/255");
        assert!(mean <= 0.4, "{name}: mean error {mean}/255");
    }
}

#[test]
fn steep_curves_bake_at_65_and_stay_within_the_bound() {
    let steep = ColorStack::default().with_curve(
        CurveChannel::Green,
        curve(&[(0, 0), (4_500, 1_000), (5_500, 9_000), (10_000, 10_000)]),
    );
    assert_eq!(BakeSize::for_stack(&steep), BakeSize::N65);
    let (max, mean, _) = lattice_error(&steep);
    assert!(max <= 2.0 && mean <= 0.4, "{max} {mean}");
    // The same curve switched off, and a gentle one, stay at 33³.
    let off = ColorStack {
        sections_enabled: steep.sections_enabled.with(Section::Curves, false),
        ..steep.clone()
    };
    assert_eq!(BakeSize::for_stack(&off), BakeSize::N33);
    let gentle = ColorStack::default().with_curve(CurveChannel::Luma, s_curve());
    assert_eq!(BakeSize::for_stack(&gentle), BakeSize::N33);
    // A Luma curve and a channel curve that are gentle alone but steep
    // one after the other.
    let stacked = ColorStack::default()
        .with_curve(
            CurveChannel::Luma,
            curve(&[(0, 0), (4_000, 2_000), (6_000, 8_000), (10_000, 10_000)]),
        )
        .with_curve(
            CurveChannel::Red,
            curve(&[(0, 0), (4_000, 2_000), (6_000, 8_000), (10_000, 10_000)]),
        );
    assert_eq!(BakeSize::for_stack(&stacked), BakeSize::N65);
}

#[test]
fn greys_never_shift_under_any_hsl_setting() {
    let mut s = ColorStack::default();
    for (i, band) in HueBand::ALL.into_iter().enumerate() {
        let v = if i % 2 == 0 { 100 } else { -100 };
        for channel in HslChannel::ALL {
            s = s.with(hsl(band, channel), v);
        }
    }
    for v in 0..=255u8 {
        let grey = unit([v; 3]);
        assert_eq!(eval_pixel(&s, grey), grey, "grey {v}");
    }
    // Nearly grey pixels barely move (the chroma fade).
    let near = unit([128, 130, 128]);
    let out = eval_pixel(&s, near);
    for i in 0..3 {
        assert!((out[i] - near[i]).abs() < 0.5 / 255.0, "{near:?} → {out:?}");
    }
}

#[test]
fn band_weights_sum_to_one_and_the_hue_wrap_is_continuous() {
    for i in 0..7200 {
        let w = band_weights(i as f32 / 20.0);
        let sum: f32 = w.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5);
    }
    // Saturated colours on both sides of 0°/360° grade alike: a hue just
    // past red toward magenta and one just past it toward orange.
    let s = ColorStack::default()
        .with(hsl(HueBand::Red, HslChannel::Hue), 80)
        .with(hsl(HueBand::Magenta, HslChannel::Saturation), -70)
        .with(hsl(HueBand::Red, HslChannel::Lightness), 30);
    let below = eval_pixel(&s, [0.9, 0.1, 0.1005]);
    let above = eval_pixel(&s, [0.9, 0.1005, 0.1]);
    for i in 0..3 {
        assert!((below[i] - above[i]).abs() < 1e-3, "{below:?} {above:?}");
    }
    // A sweep around the whole circle has no jumps.
    let mut prev: Option<[f64; 3]> = None;
    for i in 0..=3600 {
        let h = i as f32 / 10.0;
        let c = hue_color(h);
        let out = eval_pixel(&s, c);
        if let Some(p) = prev {
            let jump = (0..3)
                .map(|k| (out[k] - p[k] as f32).abs())
                .fold(0.0f32, f32::max);
            assert!(jump < 0.01, "jump {jump} at {h}°");
        }
        prev = Some(out.map(|v| v as f64));
    }
}

/// A saturated colour of HSV hue `h` degrees, value 0.9, saturation 0.8.
fn hue_color(h: f32) -> [f32; 3] {
    let h = h.rem_euclid(360.0) / 60.0;
    let (v, s) = (0.9f32, 0.8f32);
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [r + m, g + m, b + m]
}

#[test]
fn hsl_saturation_minus_100_greys_its_band_and_hue_keeps_luma() {
    let s = ColorStack::default().with(hsl(HueBand::Green, HslChannel::Saturation), -100);
    let out = eval_pixel(&s, [0.1, 0.9, 0.1]);
    assert!((out[0] - out[1]).abs() < 1e-5 && (out[1] - out[2]).abs() < 1e-5);
    // Other bands stay.
    assert_eq!(eval_pixel(&s, [0.9, 0.1, 0.1]), [0.9, 0.1, 0.1]);
    for band in HueBand::ALL {
        let s = ColorStack::default().with(hsl(band, HslChannel::Hue), 100);
        for h in (0..360).step_by(7) {
            let c = hue_color(h as f32);
            let out = eval_pixel(&s, c);
            assert!((luma(out) - luma(c)).abs() < 1e-5, "{band:?} {h}°");
        }
    }
}

#[test]
fn puck_moves_keep_luma() {
    for wheel in WheelId::ALL {
        for (x, y) in [(1000, 0), (0, -1000), (-600, 800), (250, 250)] {
            let s = ColorStack::default().with_puck(wheel, x, y);
            for px in samples().step_by(97) {
                let c = unit(px);
                let out = eval_pixel(&s, c);
                assert!(
                    (luma(out) - luma(c)).abs() < 1e-5,
                    "{wheel:?} ({x}, {y}) on {px:?}"
                );
            }
        }
    }
    for (x, y) in [(1000, 0), (-707, -707)] {
        assert!(luma(puck_vector(x, y)).abs() < 1e-6);
    }
    // The wheels weigh their ranges: Shadows moves black most, Highlights
    // white, Midtones mid-grey, and none of them moves the other ends.
    let tint = |wheel, c: [f32; 3]| {
        let out = eval_pixel(&ColorStack::default().with_puck(wheel, 1000, 0), c);
        out[2] - c[2]
    };
    assert!(tint(WheelId::Shadows, [0.0; 3]) > 0.05);
    assert!(tint(WheelId::Shadows, [1.0; 3]).abs() < 1e-6);
    assert!(tint(WheelId::Highlights, [1.0; 3]) > 0.15);
    assert!(tint(WheelId::Highlights, [0.0; 3]).abs() < 1e-6);
    assert!(tint(WheelId::Midtones, [0.5; 3]) > 0.05);
    assert!(tint(WheelId::Midtones, [0.0; 3]).abs() < 1e-6);
}

#[test]
fn luminance_sliders_and_strength_follow_lift_gamma_gain() {
    let at = |id, v, x: f32| eval_pixel(&ColorStack::default().with(id, v), [x; 3])[0];
    let lift = ColorParamId::WheelLuma(WheelId::Shadows);
    assert!((at(lift, 100, 0.0) - 0.1).abs() < 1e-6);
    assert!((at(lift, 100, 1.0) - 1.0).abs() < 1e-6);
    let gain = ColorParamId::WheelLuma(WheelId::Highlights);
    assert!((at(gain, -100, 1.0) - 0.75).abs() < 1e-6);
    assert_eq!(at(gain, -100, 0.0), 0.0);
    let gamma = ColorParamId::WheelLuma(WheelId::Midtones);
    assert!((at(gamma, 100, 0.5) - 0.595).abs() < 0.002);
    assert!((at(gamma, -100, 0.5) - 0.398).abs() < 0.002);
    assert_eq!(at(gamma, 100, 0.0), 0.0);
    let offset = ColorParamId::WheelLuma(WheelId::Offset);
    assert!((at(offset, -100, 0.4) - 0.3).abs() < 1e-6);
    // Strength scales all of it; 0 is no change at all.
    let s = ColorStack::default()
        .with(gain, -100)
        .with_puck(WheelId::Offset, 500, 500);
    let half = s.with(ColorParamId::WheelStrength, 50);
    let full = eval_pixel(&s, [0.8; 3]);
    let halfway = eval_pixel(&half, [0.8; 3]);
    for i in 0..3 {
        assert!((halfway[i] - 0.8 - (full[i] - 0.8) / 2.0).abs() < 1e-5);
    }
    let none = s.with(ColorParamId::WheelStrength, 0);
    assert!(none.is_identity());
    assert_eq!(eval_pixel(&none, [0.8, 0.3, 0.1]), [0.8, 0.3, 0.1]);
}

#[test]
fn curves_apply_luma_first_then_each_channel() {
    let lifted = curve(&[(0, 0), (5_000, 7_000), (10_000, 10_000)]);
    let s = ColorStack::default().with_curve(CurveChannel::Luma, lifted.clone());
    let out = eval_pixel(&s, [0.5, 0.5, 0.2]);
    assert!((out[0] - 0.7).abs() < 2e-3 && (out[1] - 0.7).abs() < 2e-3);
    let both = s.with_curve(CurveChannel::Blue, curve(&[(0, 10_000), (10_000, 0)]));
    let out2 = eval_pixel(&both, [0.5, 0.5, 0.5]);
    assert!((out2[2] - 0.3).abs() < 2e-3, "{out2:?}");
    // Values past 0–1 from earlier stages are clamped before the lookup.
    let bright = both.with(ColorParamId::Exposure, 100);
    let out3 = eval_pixel(&bright, [0.9, 0.9, 0.9]);
    assert!(out3[2].abs() < 1e-6 && (out3[0] - 1.0).abs() < 1e-6);
    // A flat curve gives one level.
    let flat =
        ColorStack::default().with_curve(CurveChannel::Luma, curve(&[(0, 4_000), (10_000, 4_000)]));
    assert!((eval_pixel(&flat, [0.9, 0.1, 0.5])[1] - 0.4).abs() < 1e-6);
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// The corner property (CL-015): every wheel at the rim with its
    /// luminance at an end, on top of HSL extremes, curves and Light and
    /// Color extremes, stays finite on any input.
    #[test]
    fn every_corner_stays_finite(
        pucks in proptest::collection::vec((-1i16..=1, -1i16..=1), 4),
        lumas in proptest::collection::vec(prop::bool::ANY, 4),
        bands in proptest::collection::vec((0usize..8, 0usize..3, prop::bool::ANY), 0..24),
        basics in proptest::collection::vec((0usize..15, prop::bool::ANY), 0..15),
        steep in prop::bool::ANY,
        px in prop::array::uniform3(0u8..=255),
    ) {
        let mut s = ColorStack::default();
        for (i, wheel) in WheelId::ALL.into_iter().enumerate() {
            let (x, y) = pucks[i];
            s = s.with_puck(wheel, x * 1000, y * 1000);
            s = s.with(ColorParamId::WheelLuma(wheel), if lumas[i] { 100 } else { -100 });
        }
        for (b, c, up) in bands {
            s = s.with(hsl(HueBand::ALL[b], HslChannel::ALL[c]), if up { 100 } else { -100 });
        }
        for (i, up) in basics {
            let spec = ColorParamId::ALL[i].spec();
            s = s.with(ColorParamId::ALL[i], if up { spec.max } else { spec.min });
        }
        if steep {
            s = s.with_curve(
                CurveChannel::Luma,
                curve(&[(0, 10_000), (4_900, 9_000), (5_100, 0), (10_000, 1_000)]),
            );
        }
        let out = eval_pixel(&s, unit(px));
        prop_assert!(out.iter().all(|v| v.is_finite()), "{out:?}");
        let lattice = bake_with(&s, BakeSize::N33, 1);
        prop_assert!(lattice.lookup(px).iter().all(|v| v.is_finite()));
    }
}

#[test]
fn switches_and_digests_cover_the_new_sections() {
    let s = ColorStack::default()
        .with(hsl(HueBand::Yellow, HslChannel::Hue), 50)
        .with_curve(CurveChannel::Red, s_curve())
        .with_puck(WheelId::Midtones, 300, 300);
    let px = [0.8, 0.7, 0.2];
    assert_ne!(eval_pixel(&s, px), px);
    let all_off = ColorStack {
        sections_enabled: s
            .sections_enabled
            .with(Section::Hsl, false)
            .with(Section::Curves, false)
            .with(Section::Wheels, false),
        ..s.clone()
    };
    assert!(all_off.is_identity());
    assert_eq!(eval_pixel(&all_off, px), px);
    // Every new value changes the digest while its section is on, and
    // none does while it is off.
    let d = |s: &ColorStack| ColorDigest::of(s, BakeSize::N33);
    let variants = [
        s.with(hsl(HueBand::Yellow, HslChannel::Hue), 51),
        s.with_curve(CurveChannel::Red, Curve::identity()),
        s.with_puck(WheelId::Midtones, 300, 301),
        s.with(ColorParamId::WheelLuma(WheelId::Offset), 1),
        s.with(ColorParamId::WheelStrength, 99),
    ];
    for v in &variants {
        assert_ne!(d(v), d(&s), "{v:?}");
    }
    let off = |v: &ColorStack| ColorStack {
        sections_enabled: all_off.sections_enabled,
        ..v.clone()
    };
    for v in &variants {
        assert_eq!(d(&off(v)), d(&all_off));
    }
}

#[test]
fn the_probe_averages_the_hsl_input_of_opaque_pixels() {
    // An orange 5×5 patch with one transparent blue pixel in it.
    let mut rows = vec![vec![[230, 120, 30, 255]; 7]; 7];
    rows[3][3] = [0, 0, 255, 0];
    let rows: Vec<&[[u8; 4]]> = rows.iter().map(Vec::as_slice).collect();
    let pic = picture(&rows);
    let plain = ColorStack::default();
    let v = probe_hsl_input(&plain, &pic, 3, 3).unwrap();
    assert!((v[0] - 230.0 / 255.0).abs() < 1e-6 && (v[2] - 30.0 / 255.0).abs() < 1e-6);
    assert_eq!(hsl_band_at(v), Some(HueBand::Orange));
    // It samples after wheels and curves: a blue curve that lifts blue a
    // lot turns the sampled colour toward red.
    let curved = plain.with_curve(CurveChannel::Green, curve(&[(0, 0), (10_000, 2_000)]));
    let w = probe_hsl_input(&curved, &pic, 0, 0).unwrap();
    assert!(w[1] < 0.1, "{w:?}");
    assert_eq!(hsl_band_at(w), Some(HueBand::Red));
    // Outside the picture, and a window with nothing opaque, give nothing.
    assert_eq!(probe_hsl_input(&plain, &pic, 7, 0), None);
    let clear = picture(&[&[[10, 10, 10, 0]; 3]]);
    assert_eq!(probe_hsl_input(&plain, &clear, 1, 0), None);
    // Greys and near-black name no band (CL-013).
    assert_eq!(hsl_band_at([0.5, 0.5, 0.5]), None);
    assert_eq!(hsl_band_at([0.02, 0.0, 0.0]), None);
}

/// FreeCut's test chart: a hue sweep across, a grey ramp down the middle
/// band, skin-like and saturated patches along the bottom (as in
/// `tests/color.rs`).
fn chart() -> Picture {
    let (w, h) = (96u32, 48u32);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let px = if y < 16 {
                let c = hue_color(x as f32 / w as f32 * 360.0);
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

/// Checked-in PNGs of the test chart through each new section's extremes.
/// `FREECUT_BLESS_GOLDEN=1` rewrites them.
#[test]
fn k2_extremes_match_their_golden_pictures() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/color");
    let bless = std::env::var_os("FREECUT_BLESS_GOLDEN").is_some();
    if bless {
        std::fs::create_dir_all(&dir).unwrap();
    }
    let source = chart();
    let cases: Vec<(&str, ColorStack)> = vec![
        (
            "hsl_orange_hue_plus",
            ColorStack::default().with(hsl(HueBand::Orange, HslChannel::Hue), 100),
        ),
        (
            "hsl_blue_saturation_minus",
            ColorStack::default().with(hsl(HueBand::Blue, HslChannel::Saturation), -100),
        ),
        (
            "hsl_green_lightness_plus",
            ColorStack::default().with(hsl(HueBand::Green, HslChannel::Lightness), 100),
        ),
        (
            "curves_s_curve",
            ColorStack::default().with_curve(CurveChannel::Luma, s_curve()),
        ),
        (
            "curves_red_lift",
            ColorStack::default().with_curve(
                CurveChannel::Red,
                curve(&[(0, 0), (4_000, 5_500), (10_000, 10_000)]),
            ),
        ),
        (
            "wheels_teal_shadows_warm_highlights",
            ColorStack::default()
                .with_puck(WheelId::Shadows, 700, -700)
                .with_puck(WheelId::Highlights, -500, 600),
        ),
        (
            "wheels_luma_extremes",
            ColorStack::default()
                .with(ColorParamId::WheelLuma(WheelId::Shadows), 100)
                .with(ColorParamId::WheelLuma(WheelId::Midtones), -100)
                .with(ColorParamId::WheelLuma(WheelId::Highlights), -100),
        ),
    ];
    for (name, s) in cases {
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

/// HSL, a curve and keyed pucks on a still, rendered by the preview's
/// compositor and by the exporter: the same bytes on every frame, and the
/// keyed puck really moves the grade.
#[test]
fn a_k2_grade_exports_exactly_as_previewed() {
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
    for edit in [
        ColorEdit::SetParams(vec![
            (hsl(HueBand::Orange, HslChannel::Saturation), -40),
            (ColorParamId::WheelLuma(WheelId::Midtones), 25),
        ]),
        ColorEdit::SetCurve {
            channel: CurveChannel::Luma,
            curve: s_curve(),
        },
    ] {
        p.apply(EditRequest::EditColor {
            seq,
            clips: vec![clip],
            edit,
        });
    }
    let puck = PropPath::ColorPuck(WheelId::Highlights);
    p.key(clip, puck, f(0), PropValue::Puck { x: -900, y: 0 });
    p.key(clip, puck, f(29), PropValue::Puck { x: 900, y: 0 });

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
    for k in [0u64, 10, 20, 29] {
        let exported = export.render_frame(k).unwrap().clone();
        let plan = p.plan_at(clock.frame_time(k).unwrap());
        let previewed = preview.render(&plan, 160, 90, &mut preview_images);
        assert_eq!(exported, previewed, "frame {k}");
        frames.push(exported);
    }
    // The bright grey ramp turns from yellow-ish (−Cb) to blue (+Cb); a
    // puck along Cb leaves red alone and pays for blue with green.
    let start = frames[0].pixel(140, 45).unwrap();
    let end = frames[3].pixel(140, 45).unwrap();
    assert!(
        end[2] > start[2] && end[1] < start[1],
        "{start:?} → {end:?}"
    );
    assert_eq!(end[0], start[0]);
}
