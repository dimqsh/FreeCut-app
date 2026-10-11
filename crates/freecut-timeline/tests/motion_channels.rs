//! The seeded and layout-relative channels (TX-056, FX-030): Scatter,
//! Spread, Word rise, Fall away, Shake, Flicker, Wiggle, Sparkle, 3D flip
//! in and Jitter shake, built here exactly as their files in `resources/`
//! define them. Golden samples, determinism by seed and unit, the rest
//! rules, Intensity, the photosensitivity checker and the catalogue's
//! cached availability.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

use std::sync::Arc;

use freecut_timeline::glam::DVec2;
use freecut_timeline::model::{
    AnimOrder, AnimUnit, Direction, EaseCurve, EaseDir, Interp, PerMille, PresetId, Slot,
};
use freecut_timeline::motion::{
    Availability, BlockOffsets, Channel, ChannelProp, ChannelUnit, Curve, CurvePoint, MotionScale,
    ParamDefaults, Photosensitivity, Pivot, PresetCatalog, PresetKind, PresetManifest, PresetTable,
    UnitContext, is_photosafe, photosensitivity, pose_with,
};
use freecut_timeline::time::{TICKS_PER_SECOND, Tick};

/// Lengths read in their own units: em on text, se on clips.
const UNITS: MotionScale = MotionScale {
    se_px: 1.0,
    em_px: 1.0,
    travel_px: 1.0,
};

fn ms(v: i64) -> Tick {
    Tick::from_raw(TICKS_PER_SECOND / 1000 * v)
}

fn ch(property: ChannelProp, curve: Curve, amplitude: f64, unit: ChannelUnit, k: bool) -> Channel {
    Channel {
        property,
        curve,
        amplitude,
        unit,
        scales_with_intensity: k,
    }
}

fn flat() -> Curve {
    Curve::sampled(vec![CurvePoint { t: 0.0, v: 1.0 }], false).unwrap()
}

fn ease(curve: EaseCurve, dir: EaseDir) -> Interp {
    Interp::Ease { curve, dir }
}

#[allow(clippy::too_many_arguments)]
fn manifest(
    id: &str,
    kind: PresetKind,
    units: Option<Vec<AnimUnit>>,
    directions: Option<Vec<Direction>>,
    duration_ms: i64,
    easing: Interp,
    defaults: ParamDefaults,
    channels: Vec<Channel>,
) -> PresetManifest {
    PresetManifest {
        id: PresetId::new(id),
        kind,
        units,
        directions,
        default_duration: ms(duration_ms),
        easing,
        pivot: Pivot::Anchor,
        channels,
        defaults,
    }
}

fn defaults(overlap: u16, order: AnimOrder) -> ParamDefaults {
    ParamDefaults {
        overlap: PerMille::new(overlap),
        order,
        pace: None,
    }
}

/// The ten presets that waited for these channels, as their files say.
fn presets() -> Vec<PresetManifest> {
    use AnimUnit::{Letter, Line, Word};
    use ChannelProp as P;
    use ChannelUnit as U;
    let quad_out = ease(EaseCurve::Quad, EaseDir::Out);
    let none = ParamDefaults::default();
    vec![
        manifest(
            "fc.in.scatter",
            PresetKind::Entrance,
            Some(vec![Letter]),
            None,
            1000,
            quad_out,
            defaults(800, AnimOrder::Random),
            vec![
                ch(P::Opacity, Curve::Eased, -1.0, U::Ratio, false),
                ch(P::RandomOffset, Curve::Eased, 1.5, U::Em, true),
                ch(P::RandomRotate, Curve::Eased, 40.0, U::Deg, true),
            ],
        ),
        manifest(
            "fc.in.spread",
            PresetKind::Entrance,
            Some(vec![Letter]),
            None,
            600,
            quad_out,
            defaults(1000, AnimOrder::Forward),
            vec![
                ch(P::Opacity, Curve::Eased, -1.0, U::Ratio, false),
                ch(P::TowardLineCenter, Curve::Eased, 0.6, U::Ratio, true),
            ],
        ),
        manifest(
            "fc.in.word-rise",
            PresetKind::Entrance,
            Some(vec![Word, Line]),
            None,
            700,
            ease(EaseCurve::Cubic, EaseDir::Out),
            defaults(500, AnimOrder::Forward),
            vec![ch(P::LineRise, Curve::Eased, 1.0, U::Ratio, false)],
        ),
        manifest(
            "fc.out.fall-away",
            PresetKind::Exit,
            Some(vec![Letter, Word]),
            None,
            900,
            ease(EaseCurve::Quad, EaseDir::In),
            defaults(600, AnimOrder::Random),
            vec![
                ch(P::TranslateY, Curve::Eased, 2.0, U::Em, true),
                ch(
                    P::Opacity,
                    Curve::sampled(
                        vec![
                            CurvePoint { t: 0.0, v: 0.0 },
                            CurvePoint { t: 0.6, v: 0.0 },
                            CurvePoint { t: 1.0, v: 1.0 },
                        ],
                        false,
                    )
                    .unwrap(),
                    -1.0,
                    U::Ratio,
                    false,
                ),
                ch(P::RandomRotate, Curve::Eased, 30.0, U::Deg, true),
            ],
        ),
        manifest(
            "fc.loop.shake",
            PresetKind::Loop,
            None,
            None,
            1000,
            Interp::Linear,
            none,
            vec![
                ch(P::JitterX, flat(), 0.06, U::Em, true),
                ch(P::JitterY, flat(), 0.06, U::Em, true),
            ],
        ),
        manifest(
            "fc.loop.flicker",
            PresetKind::Loop,
            None,
            None,
            1000,
            Interp::Linear,
            none,
            vec![ch(P::FlickerOpacity, flat(), 0.6, U::Ratio, true)],
        ),
        manifest(
            "fc.loop.wiggle",
            PresetKind::Loop,
            Some(vec![Letter]),
            None,
            1000,
            Interp::Linear,
            defaults(0, AnimOrder::Forward),
            vec![ch(P::JitterRotate, flat(), 6.0, U::Deg, true)],
        ),
        manifest(
            "fc.loop.sparkle",
            PresetKind::Loop,
            Some(vec![Letter]),
            None,
            2000,
            Interp::Linear,
            defaults(0, AnimOrder::Forward),
            vec![ch(P::SparkleScale, flat(), 0.2, U::Ratio, true)],
        ),
        manifest(
            "fc.clip.in.flip",
            PresetKind::Entrance,
            None,
            Some(vec![
                Direction::Left,
                Direction::Right,
                Direction::Up,
                Direction::Down,
            ]),
            500,
            quad_out,
            none,
            vec![ch(P::TurnFromSide, Curve::Eased, 90.0, U::Deg, false)],
        ),
        manifest(
            "fc.clip.loop.jitter",
            PresetKind::Loop,
            None,
            None,
            1000,
            Interp::Linear,
            none,
            vec![
                ch(P::JitterX, flat(), 0.01, U::Se, true),
                ch(P::JitterY, flat(), 0.01, U::Se, true),
                ch(P::JitterRotate, flat(), 1.5, U::Deg, true),
            ],
        ),
    ]
}

fn preset(id: &str) -> PresetManifest {
    presets().into_iter().find(|m| m.id.as_str() == id).unwrap()
}

/// The slot a preset's table describes.
fn main_slot(m: &PresetManifest) -> Slot {
    match m.kind {
        PresetKind::Entrance => Slot::Entrance,
        PresetKind::Exit => Slot::Exit,
        _ => Slot::Continuous,
    }
}

fn unit(index: u32) -> UnitContext {
    UnitContext {
        index,
        ..UnitContext::BLOCK
    }
}

fn pose(m: &PresetManifest, seed: u32, intensity: u16, p: f64, u: &UnitContext) -> BlockOffsets {
    let mut spec = m.default_spec();
    spec.params.seed = seed;
    spec.params.intensity = PerMille::new(intensity);
    pose_with(m, &spec, main_slot(m), p, &UNITS, u)
}

fn line(o: &BlockOffsets) -> String {
    format!(
        "t=({:.4},{:.4}) s=({:.4},{:.4}) r={:.4} turn=({:.4},{:.4}) a={:.4} clip={}",
        o.translate.x,
        o.translate.y,
        o.scale.x,
        o.scale.y,
        o.rotation,
        o.turn.x,
        o.turn.y,
        o.opacity,
        o.line_clip
    )
}

/// The golden poses: seed 0, Intensity 50, units 0 and 3 (unit context
/// with its line centre 2 em to the left and 0.5 em down, lines 1.25 em
/// tall), at 0/25/50/75/100 % of the slot or cycle.
fn golden_text() -> String {
    let mut out = String::new();
    for m in presets() {
        for idx in [0u32, 3] {
            let u = UnitContext {
                index: idx,
                to_line_center: DVec2::new(-2.0, 0.5),
                line_height: Some(1.25),
                ..UnitContext::BLOCK
            };
            for p in [0.0, 0.25, 0.5, 0.75, 1.0] {
                let o = pose(&m, 0, 500, p, &u);
                out.push_str(&format!("{} u{idx} {p}: {}\n", m.id, line(&o)));
            }
        }
    }
    out.replace("-0.0000", "0.0000")
}

const GOLDEN: &str = include_str!("golden/motion_channels.txt");

#[test]
fn golden_samples_of_the_new_channels() {
    let got = golden_text();
    if std::env::var_os("FREECUT_BLESS_GOLDEN").is_some() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/golden/motion_channels.txt"
        );
        std::fs::write(path, &got).unwrap();
        return;
    }
    pretty_assertions::assert_eq!(got, GOLDEN);
}

#[test]
fn seeded_channels_are_deterministic_by_seed_and_unit() {
    for id in [
        "fc.in.scatter",
        "fc.out.fall-away",
        "fc.loop.shake",
        "fc.loop.wiggle",
        "fc.loop.sparkle",
        "fc.clip.loop.jitter",
    ] {
        let m = preset(id);
        assert!(m.uses_seed(), "{id}");
        // Sampled densely, so a sparkle swell can't hide between samples.
        let track = |seed: u32, index: u32| -> Vec<BlockOffsets> {
            (0..400)
                .map(|i| pose(&m, seed, 500, f64::from(i) / 400.0, &unit(index)))
                .collect()
        };
        assert_eq!(track(5, 2), track(5, 2), "{id}: same seed, same motion");
        assert_ne!(
            track(5, 2),
            track(6, 2),
            "{id}: a new seed moves differently"
        );
        assert_ne!(track(5, 2), track(5, 3), "{id}: units move differently");
    }
    // The flicker's dips move with the seed.
    let m = preset("fc.loop.flicker");
    let dips = |seed: u32| -> Vec<bool> {
        (0..1000)
            .map(|i| pose(&m, seed, 500, f64::from(i) / 1000.0, &unit(0)).opacity < 0.5)
            .collect()
    };
    assert_eq!(dips(1), dips(1));
    assert_ne!(dips(1), dips(2));
    // Unseeded presets don't use the seed.
    assert!(!preset("fc.in.spread").uses_seed());
    assert!(!preset("fc.clip.in.flip").uses_seed());
}

#[test]
fn ins_end_at_rest_outs_start_at_rest_and_loops_are_seamless() {
    for m in presets() {
        for (intensity, seed) in [(0, 0), (500, 0), (1000, 0), (500, 11), (1000, 11)] {
            for idx in [0, 1, 7] {
                let u = UnitContext {
                    index: idx,
                    to_line_center: DVec2::new(3.0, -1.0),
                    line_height: Some(1.4),
                    ..UnitContext::BLOCK
                };
                let mut spec = m.default_spec();
                spec.params.intensity = PerMille::new(intensity);
                spec.params.seed = seed;
                let at = |slot, p| pose_with(&m, &spec, slot, p, &UNITS, &u);
                let what = format!("{} at intensity {intensity}, seed {seed}, unit {idx}", m.id);
                match m.kind {
                    PresetKind::Entrance => {
                        assert!(at(Slot::Entrance, 1.0).is_rest(1e-9), "{what}: In");
                        assert!(at(Slot::Exit, 0.0).is_rest(1e-9), "{what}: Out");
                    }
                    PresetKind::Exit => assert!(at(Slot::Exit, 0.0).is_rest(1e-9), "{what}"),
                    _ => {
                        let (a, b) = (at(Slot::Continuous, 0.0), at(Slot::Continuous, 1.0));
                        assert_eq!(a, b, "{what}: periodic");
                        let c = at(Slot::Continuous, 1.0 - 1e-9);
                        assert!(
                            (a.translate - c.translate).length() < 1e-6
                                && (a.rotation - c.rotation).abs() < 1e-5
                                && (a.scale - c.scale).length() < 1e-6
                                && (a.opacity - c.opacity).abs() < 1e-6,
                            "{what}: seamless"
                        );
                    }
                }
                // Finite everywhere.
                for i in 0..=100 {
                    let o = at(main_slot(&m), f64::from(i) / 100.0);
                    assert!(o.translate.is_finite() && o.scale.is_finite());
                    assert!(o.rotation.is_finite() && o.opacity.is_finite());
                }
            }
        }
    }
}

#[test]
fn intensity_scales_the_seeded_and_line_channels_linearly() {
    for m in presets() {
        let u = UnitContext {
            index: 4,
            to_line_center: DVec2::new(-1.5, 0.25),
            ..UnitContext::BLOCK
        };
        for i in 0..=20 {
            let p = f64::from(i) / 20.0;
            let (k0, k1, k2) = (
                pose(&m, 3, 0, p, &u),
                pose(&m, 3, 500, p, &u),
                pose(&m, 3, 1000, p, &u),
            );
            let flagged = |prop: ChannelProp| {
                m.channels
                    .iter()
                    .any(|c| c.property == prop && c.scales_with_intensity)
            };
            // Offsets and turns from flagged channels: 0, x, 2x.
            if [
                ChannelProp::RandomOffset,
                ChannelProp::TowardLineCenter,
                ChannelProp::JitterX,
                ChannelProp::JitterY,
            ]
            .into_iter()
            .any(flagged)
                && !flagged(ChannelProp::TranslateY)
            {
                assert!(k0.translate.length() < 1e-12, "{} at {p}", m.id);
                assert!((k2.translate - 2.0 * k1.translate).length() < 1e-9);
            }
            if [ChannelProp::RandomRotate, ChannelProp::JitterRotate]
                .into_iter()
                .any(flagged)
            {
                assert!(k0.rotation.abs() < 1e-12);
                assert!((k2.rotation - 2.0 * k1.rotation).abs() < 1e-9);
            }
            if flagged(ChannelProp::SparkleScale) {
                assert!((k0.scale.x - 1.0).abs() < 1e-12);
                assert!(((k2.scale.x - 1.0) - 2.0 * (k1.scale.x - 1.0)).abs() < 1e-9);
            }
            // Turns and line rises have no Intensity.
            if m.channels.iter().all(|c| !c.scales_with_intensity) {
                assert_eq!(k0, k1);
                assert_eq!(k1, k2);
            }
        }
    }
}

#[test]
fn scatter_starts_within_its_amplitude_and_turn() {
    let m = preset("fc.in.scatter");
    for idx in 0..50 {
        let o = pose(&m, 9, 500, 0.0, &unit(idx));
        let d = o.translate.length();
        assert!((0.75 - 1e-9..=1.5 + 1e-9).contains(&d), "{idx}: {d}");
        assert!(o.rotation.abs() <= 40.0, "{idx}: {}", o.rotation);
        assert_eq!(o.opacity, 0.0);
    }
    // Fall away turns up to ±30° by its end, falling 2 em.
    let m = preset("fc.out.fall-away");
    for idx in 0..50 {
        let o = pose(&m, 9, 500, 1.0, &unit(idx));
        assert!(o.rotation.abs() <= 30.0);
        assert!((o.translate.y - 2.0).abs() < 1e-12);
    }
}

#[test]
fn spread_and_word_rise_use_the_line_geometry_the_renderer_supplies() {
    let spread = preset("fc.in.spread");
    assert!(spread.needs_line_geometry());
    let u = UnitContext {
        to_line_center: DVec2::new(10.0, -2.0),
        ..UnitContext::BLOCK
    };
    // At the start: 60 % of the way to the line centre, at (1 − 0.6) of
    // its distance; Intensity 100 pulls it all the way.
    let o = pose(&spread, 0, 500, 0.0, &u);
    assert!(o.translate.abs_diff_eq(DVec2::new(6.0, -1.2), 1e-12));
    let o = pose(&spread, 0, 1000, 0.0, &u);
    assert!(o.translate.abs_diff_eq(DVec2::new(12.0, -2.4), 1e-12));
    // Without geometry (a whole block) nothing moves but the fade.
    assert_eq!(pose(&spread, 0, 500, 0.0, &unit(0)).translate, DVec2::ZERO);

    let rise = preset("fc.in.word-rise");
    assert!(rise.needs_line_geometry());
    let u = UnitContext {
        line_height: Some(80.0),
        ..UnitContext::BLOCK
    };
    let o = pose(&rise, 0, 500, 0.0, &u);
    assert_eq!(o.translate, DVec2::new(0.0, 80.0), "one line height below");
    assert!(o.line_clip, "clipped to its line while it rises");
    let o = pose(&rise, 0, 500, 1.0, &u);
    assert!(!o.line_clip && o.is_rest(0.0), "at rest: no clip");
    // Without a line height: 1.2 em.
    let em = MotionScale {
        em_px: 50.0,
        ..UNITS
    };
    let o = pose_with(
        &rise,
        &rise.default_spec(),
        Slot::Entrance,
        0.0,
        &em,
        &UnitContext::BLOCK,
    );
    assert!((o.translate.y - 60.0).abs() < 1e-9);
}

#[test]
fn the_3d_flip_turns_about_the_axis_its_direction_names() {
    let m = preset("fc.clip.in.flip");
    let start = |dir: Option<Direction>, slot: Slot, p: f64| {
        let mut spec = m.default_spec();
        spec.direction = dir;
        pose_with(&m, &spec, slot, p, &UNITS, &UnitContext::BLOCK).turn
    };
    assert_eq!(start(None, Slot::Entrance, 0.0), DVec2::new(0.0, 90.0));
    assert_eq!(
        start(Some(Direction::Left), Slot::Entrance, 0.0),
        DVec2::new(0.0, 90.0)
    );
    assert_eq!(
        start(Some(Direction::Right), Slot::Entrance, 0.0),
        DVec2::new(0.0, -90.0)
    );
    assert_eq!(
        start(Some(Direction::Up), Slot::Entrance, 0.0),
        DVec2::new(90.0, 0.0)
    );
    assert_eq!(
        start(Some(Direction::Down), Slot::Entrance, 0.0),
        DVec2::new(-90.0, 0.0)
    );
    // Half way, ease-out quad: a quarter of the turn left.
    assert!((start(None, Slot::Entrance, 0.5).y - 22.5).abs() < 1e-12);
    assert_eq!(start(None, Slot::Entrance, 1.0), DVec2::ZERO);
    // Its Out leaves to the right by default (FX-030): the mirrored turn.
    assert_eq!(start(None, Slot::Exit, 0.0), DVec2::ZERO);
    assert_eq!(start(None, Slot::Exit, 1.0), DVec2::new(0.0, -90.0));
    let o = pose(&m, 0, 500, 0.3, &unit(0));
    assert!(o.has_turn());
}

#[test]
fn jitter_stays_within_its_amplitude_and_turns_twelve_times_a_second() {
    let m = preset("fc.clip.loop.jitter");
    let samples: Vec<BlockOffsets> = (0..1200)
        .map(|i| pose(&m, 4, 1000, f64::from(i) / 1200.0, &unit(0)))
        .collect();
    // k = 2: ±0.02 se and ±3°.
    assert!(samples.iter().all(|o| o.translate.x.abs() <= 0.02 + 1e-12));
    assert!(samples.iter().all(|o| o.translate.y.abs() <= 0.02 + 1e-12));
    assert!(samples.iter().all(|o| o.rotation.abs() <= 3.0 + 1e-12));
    let n = samples.len();
    let turns = |f: &dyn Fn(&BlockOffsets) -> f64| {
        (0..n)
            .filter(|&i| {
                let (a, b, c) = (
                    f(&samples[(i + n - 1) % n]),
                    f(&samples[i]),
                    f(&samples[(i + 1) % n]),
                );
                (b - a) * (c - b) < 0.0
            })
            .count()
    };
    assert_eq!(turns(&|o| o.translate.x), 12);
    assert_eq!(turns(&|o| o.translate.y), 12);
    assert_eq!(turns(&|o| o.rotation), 12);
    // Brightness never moves: jitter can't flash.
    assert!(samples.iter().all(|o| o.opacity == 1.0 && o.tint == 0.0));
}

#[test]
fn sparkle_swells_each_unit_once_a_cycle_for_a_fifth_of_a_second() {
    let m = preset("fc.loop.sparkle");
    for idx in 0..8 {
        let scales: Vec<f64> = (0..2000)
            .map(|i| pose(&m, 2, 500, f64::from(i) / 2000.0, &unit(idx)).scale.x)
            .collect();
        let peak = scales.iter().copied().fold(0.0, f64::max);
        assert!(peak > 1.199 && peak <= 1.2 + 1e-12, "{idx}: {peak}");
        // 0.2 s of a 2 s cycle.
        let swelling = scales.iter().filter(|s| **s > 1.0).count();
        assert!((195..=201).contains(&swelling), "{idx}: {swelling}");
    }
}

#[test]
fn flicker_dips_for_two_frames_at_most_once_a_second() {
    let m = preset("fc.loop.flicker");
    // A 1 s cycle holds one dip of 2 frames (at 30 fps without a frame
    // rate): opacity 1 − 0.6.
    for seed in 0..10 {
        let opac: Vec<f64> = (0..3000)
            .map(|i| pose(&m, seed, 500, f64::from(i) / 3000.0, &unit(0)).opacity)
            .collect();
        let low = opac.iter().filter(|a| (**a - 0.4).abs() < 1e-12).count();
        assert!((199..=201).contains(&low), "seed {seed}: {low}");
        assert!(opac.iter().all(|a| *a == 1.0 || (*a - 0.4).abs() < 1e-12));
    }
    // With the sequence's frame: two frames at 25 fps.
    let frame = Tick::from_raw(TICKS_PER_SECOND / 25);
    let u = UnitContext {
        frame: Some(frame),
        ..UnitContext::BLOCK
    };
    let low = (0..3000)
        .filter(|i| pose(&m, 0, 500, f64::from(*i) / 3000.0, &u).opacity < 1.0)
        .count();
    assert!((239..=241).contains(&low), "{low}");
    // A cycle under a second has no room for a dip a second from the next.
    let mut spec = m.default_spec();
    spec.duration = ms(800);
    assert!((0..800).all(|i| {
        pose_with(
            &m,
            &spec,
            Slot::Continuous,
            f64::from(i) / 800.0,
            &UNITS,
            &u,
        )
        .opacity
            == 1.0
    }));
}

#[test]
fn the_checker_passes_every_new_preset_and_counts_flicker_dips() {
    for m in presets() {
        let report = photosensitivity(&m);
        assert!(report.is_safe(), "{}: {report:?}", m.id);
        assert!(is_photosafe(&m));
    }
    let flicker = photosensitivity(&preset("fc.loop.flicker"));
    assert_eq!(flicker.flash_limit, Photosensitivity::MAX_LOOP_FLASHES);
    // One dip a cycle, a second apart: a window of exactly one second may
    // catch two of them, never more, and never two flashes.
    assert!((1..=Photosensitivity::MAX_DIPS).contains(&flicker.dips_per_second));
    assert_eq!(flicker.flashes_per_second, 1);
    // A longer cycle has more dips, still a second or more apart.
    let mut long = preset("fc.loop.flicker");
    long.default_duration = ms(4000);
    let report = photosensitivity(&long);
    assert!(report.is_safe(), "{report:?}");
    assert!((1..=Photosensitivity::MAX_DIPS).contains(&report.dips_per_second));
}

#[test]
fn the_checker_refuses_fast_dips_and_strobes() {
    // A loop dipping three times a second: over the 2-dip limit.
    let dip = Curve::sampled(
        [(0.0, 0.0), (0.1, 0.0), (0.15, 1.0), (0.2, 0.0), (1.0, 0.0)]
            .into_iter()
            .map(|(t, v)| CurvePoint { t, v })
            .collect(),
        false,
    )
    .unwrap();
    let mut dips = preset("fc.loop.flicker");
    dips.channels = vec![ch(
        ChannelProp::Opacity,
        dip,
        -0.6,
        ChannelUnit::Ratio,
        true,
    )];
    dips.default_duration = ms(333);
    let report = photosensitivity(&dips);
    assert!(report.dips_per_second >= 3, "{report:?}");
    assert!(!report.is_safe());
    // An entrance that strobes white tint five times.
    let strobe = Curve::sampled(
        (0..11)
            .map(|i| CurvePoint {
                t: f64::from(i) / 10.0,
                v: if i % 2 == 0 { 0.0 } else { 1.0 },
            })
            .collect(),
        false,
    )
    .unwrap();
    let mut flash = preset("fc.clip.in.flip");
    flash.channels = vec![ch(
        ChannelProp::Tint,
        strobe,
        1.0,
        ChannelUnit::Ratio,
        false,
    )];
    flash.default_duration = ms(1000);
    let report = photosensitivity(&flash);
    assert!(report.flashes_per_second > 3, "{report:?}");
    assert!(!is_photosafe(&flash));
}

#[test]
fn the_table_caches_availability_and_knows_what_was_left_out() {
    let mut unsafe_loop = preset("fc.loop.flicker");
    unsafe_loop.id = PresetId::new("test.loop.strobe");
    unsafe_loop.channels = vec![ch(
        ChannelProp::Opacity,
        Curve::Sine,
        -1.0,
        ChannelUnit::Ratio,
        false,
    )];
    unsafe_loop.default_duration = ms(200);
    let mut all = presets();
    all.push(unsafe_loop);
    let table = PresetTable::new(all).with_unavailable([
        (
            PresetId::new("fc.in.glitch"),
            Arc::from("unknown channel \"glitch\""),
        ),
        // Loaded ids stay loaded.
        (PresetId::new("fc.in.scatter"), Arc::from("ignored")),
    ]);
    assert_eq!(
        table.availability(&PresetId::new("fc.in.scatter")),
        Availability::Ready
    );
    assert!(matches!(
        table.availability(&PresetId::new("test.loop.strobe")),
        Availability::Unsafe(p) if p.flashes_per_second > 2
    ));
    assert_eq!(
        table.availability(&PresetId::new("fc.in.glitch")),
        Availability::Unavailable(Arc::from("unknown channel \"glitch\""))
    );
    assert_eq!(
        table.availability(&PresetId::new("fc.in.nothing")),
        Availability::Unknown
    );
    assert_eq!(
        table.unavailable().collect::<Vec<_>>(),
        vec![(&PresetId::new("fc.in.glitch"), "unknown channel \"glitch\"")]
    );
    assert_eq!(table.safe_manifests().count(), presets().len());
    assert!(table.is_safe(&PresetId::new("fc.loop.flicker")));
    assert!(!table.is_safe(&PresetId::new("test.loop.strobe")));
    assert!(!table.is_safe(&PresetId::new("fc.in.glitch")));
    // Computed once and shared by clones: a precomputed table answers
    // without evaluating again (timed loosely: thousands of answers in
    // well under the cost of one check).
    let copy = table.clone();
    table.precompute();
    let started = std::time::Instant::now();
    for _ in 0..1000 {
        for m in copy.manifests() {
            assert!(copy.photosensitivity(&m.id).is_some());
        }
    }
    let cached = started.elapsed();
    let started = std::time::Instant::now();
    let _ = photosensitivity(&preset("fc.loop.flicker"));
    let once = started.elapsed();
    assert!(cached < once * 4, "cached {cached:?} vs one check {once:?}");
}

// ------------------------- brightness the renderer adds up (PS-2), cycles

/// A letter loop of `cycle_ms` whose channels each swing `amplitude`
/// once a cycle (a triangle).
fn swinging(cycle_ms: i64, channels: &[(ChannelProp, f64)]) -> PresetManifest {
    let triangle = || {
        Curve::sampled(
            vec![
                CurvePoint { t: 0.0, v: 0.0 },
                CurvePoint { t: 0.5, v: 1.0 },
                CurvePoint { t: 1.0, v: 0.0 },
            ],
            false,
        )
        .unwrap()
    };
    manifest(
        "test.loop.swing",
        PresetKind::Loop,
        Some(vec![AnimUnit::Letter]),
        None,
        cycle_ms,
        Interp::Linear,
        defaults(0, AnimOrder::Forward),
        channels
            .iter()
            .map(|&(p, a)| ch(p, triangle(), a, ChannelUnit::Ratio, false))
            .collect(),
    )
}

#[test]
fn tint_and_lightness_count_together_as_the_renderer_adds_them() {
    use freecut_timeline::motion::StyleChannel;
    let lightness = ChannelProp::Style(StyleChannel::Lightness);
    // Each alone swings 8 %, under the 10 % threshold: no flash.
    for alone in [ChannelProp::Tint, lightness] {
        let report = photosensitivity(&swinging(200, &[(alone, 0.08)]));
        assert_eq!(report.flashes_per_second, 0, "{alone:?}: {report:?}");
    }
    // Together a glyph's tint swings 16 % five times a second: a strobe.
    let both = photosensitivity(&swinging(
        200,
        &[(ChannelProp::Tint, 0.08), (lightness, 0.08)],
    ));
    assert!(both.flashes_per_second >= 4, "{both:?}");
    assert!(!both.is_safe());
}

#[test]
fn a_fast_hue_turn_counts_as_a_brightness_change() {
    use freecut_timeline::motion::StyleChannel;
    let hue = |cycle_ms| {
        let mut m = swinging(cycle_ms, &[(ChannelProp::Style(StyleChannel::Hue), 0.0)]);
        m.channels[0].curve = Curve::sampled(
            vec![CurvePoint { t: 0.0, v: 0.0 }, CurvePoint { t: 1.0, v: 1.0 }],
            false,
        )
        .unwrap();
        m.channels[0].amplitude = 360.0;
        m.channels[0].unit = ChannelUnit::Deg;
        m
    };
    // Once round the wheel every 0.2 s: blue to yellow and back, strobing.
    let report = photosensitivity(&hue(200));
    assert!(!report.is_safe(), "{report:?}");
    // Hue cycle's own 4 s turn is calm.
    let report = photosensitivity(&hue(4000));
    assert!(report.is_safe(), "{report:?}");
}

#[test]
fn the_shortest_safe_cycle_is_the_last_one_that_passes() {
    use freecut_timeline::model::AnimationSpec;
    use freecut_timeline::motion::{StyleChannel, min_safe_cycle};
    let shimmer = swinging(1500, &[(ChannelProp::Style(StyleChannel::Lightness), 0.4)]);
    let floor = min_safe_cycle(&shimmer);
    let at = |cycle: Tick| {
        let mut m = shimmer.clone();
        m.default_duration = cycle;
        photosensitivity(&m)
    };
    assert!(at(floor).is_safe(), "{:?}", at(floor));
    assert!(
        floor > AnimationSpec::MIN_DURATION && floor < ms(1500),
        "{floor:?}"
    );
    assert_eq!(
        floor.raw() % (TICKS_PER_SECOND / 100),
        0,
        "on the 10 ms grid"
    );
    // Every grid cycle from there up passes; the one below doesn't.
    let floor_ms = floor.raw() * 1000 / TICKS_PER_SECOND;
    for cycle_ms in (floor_ms..1500).step_by(10) {
        assert!(at(ms(cycle_ms)).is_safe(), "{cycle_ms} ms flashes");
    }
    assert!(!at(ms(floor_ms - 10)).is_safe());
    // At 0.1 s it would strobe.
    assert!(!at(AnimationSpec::MIN_DURATION).is_safe());
    // Motion without brightness may cycle as fast as FX-019 allows; In
    // and Out presets have no cycle.
    let float = swinging(1200, &[(ChannelProp::TranslateY, 0.3)]);
    assert_eq!(min_safe_cycle(&float), AnimationSpec::MIN_DURATION);
    // A table caches the same answer.
    let table = PresetTable::new([shimmer.clone()]);
    assert_eq!(table.min_cycle(&shimmer.id), floor);
    table.precompute();
    assert_eq!(table.min_cycle(&shimmer.id), floor);
}
