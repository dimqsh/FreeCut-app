//! The pure animation engine (FX-030, TX-022, TX-023, TX-056): golden
//! samples of the built-in manifests, rest at the slot boundaries, the
//! photosensitivity rule, distances and Pace.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

use freecut_timeline::glam::DVec2;
use freecut_timeline::model::{
    AnimUnit, AnimationSpec, Direction, EaseCurve, EaseDir, Interp, PerMille, PresetId, Slot,
    SlotWindow, Travel,
};
use freecut_timeline::motion::{
    BlockOffsets, ChannelProp, MotionScale, PresetCatalog, PresetKind, PresetTable, RestBounds,
    animation_offsets, effective_direction, effective_unit, max_flashes_per_second, pose_at,
    samples, slot_progress, travel_px,
};
use freecut_timeline::time::{FrameRate, TICKS_PER_SECOND, Tick};

const SCALE: MotionScale = MotionScale {
    se_px: 1080.0,
    em_px: 100.0,
    travel_px: 1000.0,
};

fn manifest(id: &str) -> std::sync::Arc<freecut_timeline::motion::PresetManifest> {
    PresetTable::samples().get(&PresetId::new(id)).unwrap()
}

fn ms(v: i64) -> Tick {
    Tick::from_raw(TICKS_PER_SECOND / 1000 * v)
}

/// (translate x, translate y, scale x, opacity) at 0, 25, 50, 75 and 100 %.
type Golden = [(f64, f64, f64, f64); 5];

fn check(id: &str, slot: Slot, golden: &Golden) {
    let m = manifest(id);
    let spec = m.default_spec();
    for (i, want) in golden.iter().enumerate() {
        let p = i as f64 / 4.0;
        let o = pose_at(&m, &spec, slot, p, &SCALE);
        let got = (o.translate.x, o.translate.y, o.scale.x, o.opacity);
        let close = |a: f64, b: f64| (a - b).abs() < 1e-5;
        assert!(
            close(got.0, want.0)
                && close(got.1, want.1)
                && close(got.2, want.2)
                && close(got.3, want.3),
            "{id} {slot:?} at {p}: {got:?}, want {want:?}"
        );
    }
}

#[test]
fn golden_samples_of_the_built_in_presets() {
    // Fade: opacity 0 → 1, quadratic ease-out.
    check(
        "fc.in.fade",
        Slot::Entrance,
        &[
            (0.0, 0.0, 1.0, 0.0),
            (0.0, 0.0, 1.0, 0.4375),
            (0.0, 0.0, 1.0, 0.75),
            (0.0, 0.0, 1.0, 0.9375),
            (0.0, 0.0, 1.0, 1.0),
        ],
    );
    // Its exit: the same motion backwards, easing mirrored.
    check(
        "fc.in.fade",
        Slot::Exit,
        &[
            (0.0, 0.0, 1.0, 1.0),
            (0.0, 0.0, 1.0, 0.9375),
            (0.0, 0.0, 1.0, 0.75),
            (0.0, 0.0, 1.0, 0.4375),
            (0.0, 0.0, 1.0, 0.0),
        ],
    );
    // Rise: 1 em (100 px) below, cubic ease-out; opaque from 40 %.
    check(
        "fc.in.rise",
        Slot::Entrance,
        &[
            (0.0, 100.0, 1.0, 0.0),
            (0.0, 42.1875, 1.0, 0.771484),
            (0.0, 12.5, 1.0, 1.0),
            (0.0, 1.5625, 1.0, 1.0),
            (0.0, 0.0, 1.0, 1.0),
        ],
    );
    // Slide from the left by the travel distance.
    check(
        "fc.in.slide",
        Slot::Entrance,
        &[
            (-1000.0, 0.0, 1.0, 1.0),
            (-421.875, 0.0, 1.0, 1.0),
            (-125.0, 0.0, 1.0, 1.0),
            (-15.625, 0.0, 1.0, 1.0),
            (0.0, 0.0, 1.0, 1.0),
        ],
    );
    // Its exit leaves to the right by default (FX-030).
    check(
        "fc.in.slide",
        Slot::Exit,
        &[
            (0.0, 0.0, 1.0, 1.0),
            (15.625, 0.0, 1.0, 1.0),
            (125.0, 0.0, 1.0, 1.0),
            (421.875, 0.0, 1.0, 1.0),
            (1000.0, 0.0, 1.0, 1.0),
        ],
    );
    // Pop: 0 → past 1 → 1.
    check(
        "fc.in.pop",
        Slot::Entrance,
        &[
            (0.0, 0.0, 0.0, 1.0),
            (0.0, 0.0, 0.509585, 1.0),
            (0.0, 0.0, 1.028846, 1.0),
            (0.0, 0.0, 1.129009, 1.0),
            (0.0, 0.0, 1.0, 1.0),
        ],
    );
    // Typewriter, for one letter's window: hidden until it starts.
    check(
        "fc.in.typewriter",
        Slot::Entrance,
        &[
            (0.0, 0.0, 1.0, 0.0),
            (0.0, 0.0, 1.0, 1.0),
            (0.0, 0.0, 1.0, 1.0),
            (0.0, 0.0, 1.0, 1.0),
            (0.0, 0.0, 1.0, 1.0),
        ],
    );
    // Wave, over one cycle: up a quarter em first.
    check(
        "fc.loop.wave",
        Slot::Continuous,
        &[
            (0.0, 0.0, 1.0, 1.0),
            (0.0, -25.0, 1.0, 1.0),
            (0.0, 0.0, 1.0, 1.0),
            (0.0, 25.0, 1.0, 1.0),
            (0.0, 0.0, 1.0, 1.0),
        ],
    );
}

#[test]
fn every_in_ends_at_rest_and_every_out_starts_at_rest() {
    for m in samples() {
        let mut spec = m.default_spec();
        for intensity in [0, 500, 1000] {
            spec.params.intensity = PerMille::new(intensity);
            for easing in [None, Some(Interp::BounceOut), Some(Interp::OvershootIn)] {
                spec.easing = easing;
                let at = |slot, p| pose_at(&m, &spec, slot, p, &SCALE);
                match m.kind {
                    PresetKind::Entrance => {
                        assert!(at(Slot::Entrance, 1.0).is_rest(1e-9), "{} In", m.id);
                        assert!(at(Slot::Exit, 0.0).is_rest(1e-9), "{} Out", m.id);
                    }
                    PresetKind::Exit => assert!(at(Slot::Exit, 0.0).is_rest(1e-9), "{}", m.id),
                    PresetKind::Loop => {
                        assert!(at(Slot::Continuous, 0.0).is_rest(1e-9), "{}", m.id);
                        assert!(at(Slot::Continuous, 1.0).is_rest(1e-9), "{}", m.id);
                    }
                    PresetKind::Drift | PresetKind::Combined => {}
                }
            }
        }
    }
}

#[test]
fn no_preset_flashes_more_than_three_times_a_second() {
    for m in samples() {
        let flashes = max_flashes_per_second(&m);
        assert!(flashes <= 3, "{}: {flashes} flashes", m.id);
        if m.kind == PresetKind::Loop {
            assert!(flashes <= 2, "{}: {flashes} dips", m.id);
        }
    }
    // The checker does catch a flicker: a loop dipping five times a cycle.
    let mut flicker = manifest("fc.loop.wave").as_ref().clone();
    flicker.channels[0].property = ChannelProp::Opacity;
    flicker.channels[0].unit = freecut_timeline::motion::ChannelUnit::Ratio;
    flicker.channels[0].amplitude = -0.5;
    flicker.default_duration = ms(200);
    assert!(max_flashes_per_second(&flicker) > 3);
}

#[test]
fn intensity_scales_only_the_channels_that_say_so() {
    let m = manifest("fc.in.rise");
    let mut spec = m.default_spec();
    spec.params.intensity = PerMille::new(1000);
    let o = pose_at(&m, &spec, Slot::Entrance, 0.0, &SCALE);
    assert!(
        (o.translate.y - 200.0).abs() < 1e-9,
        "k = 2 doubles the rise"
    );
    assert_eq!(o.opacity, 0.0, "the fade has no intensity");
    spec.params.intensity = PerMille::ZERO;
    let o = pose_at(&m, &spec, Slot::Entrance, 0.0, &SCALE);
    assert_eq!(o.translate.y, 0.0, "intensity 0: a pure fade");
}

#[test]
fn easing_override_replaces_the_built_in_curve() {
    let m = manifest("fc.in.fade");
    let mut spec = m.default_spec();
    spec.easing = Some(Interp::Linear);
    let o = pose_at(&m, &spec, Slot::Entrance, 0.25, &SCALE);
    assert!((o.opacity - 0.25).abs() < 1e-12);
    // On an exit the override is the exit's own curve: ease-in leaves
    // slowly.
    spec.easing = Some(Interp::Ease {
        curve: EaseCurve::Quad,
        dir: EaseDir::In,
    });
    let o = pose_at(&m, &spec, Slot::Exit, 0.5, &SCALE);
    assert!((o.opacity - 0.75).abs() < 1e-12, "{}", o.opacity);
}

#[test]
fn directions_turn_slides_and_travel_measures_from_the_edge() {
    let m = manifest("fc.in.slide");
    let mut spec = m.default_spec();
    assert_eq!(
        effective_direction(&m, &spec, Slot::Entrance),
        Some(Direction::Left)
    );
    assert_eq!(
        effective_direction(&m, &spec, Slot::Exit),
        Some(Direction::Right)
    );
    let start = |spec: &AnimationSpec| pose_at(&m, spec, Slot::Entrance, 0.0, &SCALE).translate;
    for (dir, want) in [
        (Direction::Left, DVec2::new(-1000.0, 0.0)),
        (Direction::Right, DVec2::new(1000.0, 0.0)),
        (Direction::Up, DVec2::new(0.0, -1000.0)),
        (Direction::Down, DVec2::new(0.0, 1000.0)),
    ] {
        spec.direction = Some(dir);
        assert!(start(&spec).abs_diff_eq(want, 1e-9), "{dir:?}");
    }
    // A direction the preset doesn't offer falls back to its default.
    spec.direction = Some(Direction::Clockwise);
    assert_eq!(
        effective_direction(&m, &spec, Slot::Entrance),
        Some(Direction::Left)
    );
    // From edge: just past the canvas edge, measured from the bounds.
    let rest = RestBounds {
        canvas: DVec2::new(1920.0, 1080.0),
        min: DVec2::new(800.0, 500.0),
        max: DVec2::new(1100.0, 580.0),
        em_px: 64.0,
        text: true,
    };
    assert_eq!(
        travel_px(Some(Direction::Left), Travel::FromEdge, &rest),
        1100.0
    );
    assert_eq!(
        travel_px(Some(Direction::Right), Travel::FromEdge, &rest),
        1120.0
    );
    assert_eq!(
        travel_px(Some(Direction::Up), Travel::FromEdge, &rest),
        580.0
    );
    assert_eq!(
        travel_px(Some(Direction::Down), Travel::FromEdge, &rest),
        580.0
    );
    // Already off the canvas: still moves at least its own size.
    let off = RestBounds {
        min: DVec2::new(-500.0, 0.0),
        max: DVec2::new(-300.0, 50.0),
        ..rest
    };
    assert_eq!(
        travel_px(Some(Direction::Left), Travel::FromEdge, &off),
        200.0
    );
    // Short: 3 em on text, 0.15 short edges elsewhere.
    assert_eq!(travel_px(None, Travel::Short, &rest), 192.0);
    assert!(
        (travel_px(
            None,
            Travel::Short,
            &RestBounds {
                text: false,
                ..rest
            }
        ) - 162.0)
            .abs()
            < 1e-9
    );
    // animation_offsets puts it together: the slide starts just off-canvas.
    let spec = m.default_spec();
    let window = SlotWindow {
        start: Tick::ZERO,
        len: ms(600),
    };
    let o = animation_offsets(&m, &spec, Slot::Entrance, Tick::ZERO, window, &rest);
    assert!((o.translate.x + 1100.0).abs() < 1e-9);
    let o = animation_offsets(&m, &spec, Slot::Entrance, ms(600), window, &rest);
    assert!(o.is_rest(1e-9));
}

#[test]
fn units_follow_the_preset_and_the_clip_kind() {
    let tw = manifest("fc.in.typewriter");
    let wave = manifest("fc.loop.wave");
    let mut spec = wave.default_spec();
    assert_eq!(effective_unit(&wave, &spec, true), AnimUnit::Letter);
    spec.params.unit = Some(AnimUnit::Word);
    assert_eq!(effective_unit(&wave, &spec, true), AnimUnit::Word);
    spec.params.unit = Some(AnimUnit::Line);
    assert_eq!(
        effective_unit(&wave, &spec, true),
        AnimUnit::Letter,
        "snaps back"
    );
    assert_eq!(
        effective_unit(&wave, &spec, false),
        AnimUnit::Block,
        "not text"
    );
    assert_eq!(
        effective_unit(&tw, &tw.default_spec(), true),
        AnimUnit::Letter
    );
    assert_eq!(
        effective_unit(&manifest("fc.in.fade"), &spec, true),
        AnimUnit::Block
    );
}

#[test]
fn loops_run_from_the_clip_start_plus_the_phase() {
    let m = manifest("fc.loop.wave");
    let mut spec = m.default_spec(); // 1.2 s cycle
    let window = SlotWindow {
        start: ms(500),
        len: ms(3000),
    };
    let p = |spec: &AnimationSpec, t| slot_progress(&m, spec, ms(t), window);
    assert!((p(&spec, 300) - 0.25).abs() < 1e-12);
    assert!((p(&spec, 1500) - 0.25).abs() < 1e-12, "wraps every cycle");
    // A right piece of a split at 1 s continues where the left one was.
    spec.phase = ms(1000);
    assert!((p(&spec, 0) - slot_progress(&m, &m.default_spec(), ms(1000), window)).abs() < 1e-12);
    // In and Out run across their window.
    let fade = manifest("fc.in.fade");
    let w = SlotWindow {
        start: ms(100),
        len: ms(400),
    };
    assert_eq!(slot_progress(&fade, &fade.default_spec(), ms(300), w), 0.5);
    assert_eq!(slot_progress(&fade, &fade.default_spec(), ms(0), w), 0.0);
    assert_eq!(slot_progress(&fade, &fade.default_spec(), ms(900), w), 1.0);
}

#[test]
fn rest_is_the_identity() {
    assert!(BlockOffsets::REST.is_rest(0.0));
    assert!(!BlockOffsets::REST.has_turn());
    let _ = FrameRate::new(25, 1).unwrap();
}
