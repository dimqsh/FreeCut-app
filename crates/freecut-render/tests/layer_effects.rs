//! Whole-layer animation effects in the compositor (FX-030, TX-056): the
//! blur, the wipe and the tint a frame plan's `LayerMotion` carries, layers
//! turned in 3D with perspective, per-letter tint (Shimmer's band),
//! Typewriter's caret option and the Decode step, caching, determinism
//! across threads and the time a blurred, turned 1080p layer takes.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Instant;

use common::project::{Project, ink_box};
use common::*;
use freecut_render::cpu::{Compositor, Frame, NoImages, Picture, TextAnchor, TextLayer};
use freecut_render::motion::{DecodeStep, TextMotionOptions};
use freecut_render::text::animate::{
    AnimUnit, Caret, ChannelCurve, ChannelTable, GlyphChannel, TextMotion, UnitChannels, UnitOrder,
    UnitTiming, loop_delay,
};
use freecut_render::text::{Fill, FontRegistry, TextRaster, TextRenderer, TextSpec};
use freecut_timeline::edit::TextStyleEdit;
use freecut_timeline::glam::{DMat3, DVec2, DVec3};
use freecut_timeline::model::{
    AnimUnit as ModelUnit, AnimationSpec, Direction, Interp, PerMille, PresetId, RefPx, Slot,
    TextClipData,
};
use freecut_timeline::motion::{
    Channel, ChannelProp, ChannelUnit, Curve, ParamDefaults, Pivot, PresetKind, PresetManifest,
    PresetTable, samples,
};
use freecut_timeline::plan::{
    FramePlan, LayerMotion, LayerPlan, LayerSource, PlanInputs, WIPE_FEATHER,
};
use freecut_timeline::time::{TICKS_PER_SECOND, Tick};

fn compositor() -> Compositor {
    Compositor::new(FontRegistry::bundled())
}

fn translate(x: f64, y: f64) -> DMat3 {
    DMat3::from_translation(DVec2::new(x, y))
}

/// A layer's motion with only the whole-layer effects set.
fn motion(blur_px: f64, reveal: f64, from: Direction, tint: f64) -> Option<Arc<LayerMotion>> {
    Some(Arc::new(LayerMotion {
        blur_px,
        reveal,
        reveal_from: from,
        reveal_feather: if reveal < 1.0 { WIPE_FEATHER } else { 0.0 },
        tint,
        text_reveal: 1.0,
        text: None,
    }))
}

fn with_motion(mut l: LayerPlan, m: Option<Arc<LayerMotion>>) -> LayerPlan {
    l.motion = m;
    l
}

fn px(f: &Frame, x: u32, y: u32) -> [u8; 4] {
    f.pixel(x, y).unwrap()
}

/// A 3D turn as the frame plan folds it (FX-030): about the vertical axis
/// by `turn_y`, then the horizontal one by `turn_x` (degrees), in
/// perspective with the eye `eye` px away, about the centre of a `size`
/// layer placed at `centre`.
fn turned(centre: DVec2, size: (f64, f64), turn_x: f64, turn_y: f64, eye: f64) -> DMat3 {
    let (ty, tx) = (turn_y.to_radians(), turn_x.to_radians());
    let about_vertical = DMat3::from_cols(
        DVec3::new(ty.cos(), 0.0, ty.sin() / eye),
        DVec3::Y,
        DVec3::Z,
    );
    let about_horizontal = DMat3::from_cols(
        DVec3::X,
        DVec3::new(0.0, tx.cos(), tx.sin() / eye),
        DVec3::Z,
    );
    DMat3::from_translation(centre)
        * about_horizontal
        * about_vertical
        * DMat3::from_translation(DVec2::new(-size.0 / 2.0, -size.1 / 2.0))
}

/// The image of `p` under `m`, divided by the third coordinate.
fn project(m: DMat3, p: DVec2) -> DVec2 {
    let q = m * p.extend(1.0);
    DVec2::new(q.x / q.z, q.y / q.z)
}

// --- Blur ---

/// The standard normal CDF (Abramowitz–Stegun 7.1.26, |error| < 1.5e-7).
fn phi(x: f64) -> f64 {
    let z = x.abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.327_591_1 * z);
    let poly = t
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    let erf = 1.0 - poly * (-z * z).exp();
    if x >= 0.0 {
        0.5 * (1.0 + erf)
    } else {
        0.5 * (1.0 - erf)
    }
}

fn blurred_square(blur_px: f64) -> Frame {
    let mut images = Images::default().with(1, solid(40, 40, WHITE));
    let l = with_motion(
        layer(still(1), (40, 40), translate(80.0, 80.0)),
        motion(blur_px, 1.0, Direction::Left, 0.0),
    );
    compositor().render(&plan(200, 200, vec![l]), 200, 200, &mut images)
}

fn energy(f: &Frame) -> f64 {
    f.rgba.chunks(4).map(|p| f64::from(p[0])).sum()
}

#[test]
fn blur_conserves_energy_and_spreads_by_its_radius() {
    let sharp = blurred_square(0.0);
    let soft = blurred_square(8.0);
    let (e0, e1) = (energy(&sharp), energy(&soft));
    assert!((e1 - e0).abs() / e0 < 0.005, "{e0} vs {e1}");
    // The edge profile across x = 80 is the box convolved with a Gaussian
    // of σ = radius / 2 (the text blurs' convention).
    let sigma = 4.0;
    for x in 64..100u32 {
        let c = f64::from(x) + 0.5;
        let want = 255.0 * (phi((c - 80.0) / sigma) - phi((c - 120.0) / sigma));
        let got = f64::from(px(&soft, x, 100)[0]);
        assert!((got - want).abs() <= 4.0, "x {x}: {got} vs {want}");
    }
    // Nothing reaches beyond three σ (plus a pixel).
    assert_eq!(px(&soft, 66, 100), BG);
    assert!(px(&soft, 70, 100)[0] > 0);
    // Symmetric about the square's centre.
    for d in 0..30 {
        let (a, b) = (px(&soft, 100 - d, 100)[0], px(&soft, 99 + d, 100)[0]);
        assert!(a.abs_diff(b) <= 1, "{d}: {a} {b}");
    }
}

#[test]
fn large_blurs_are_downsampled_and_still_conserve_energy() {
    let sharp = blurred_square(0.0);
    let soft = blurred_square(40.0);
    let (e0, e1) = (energy(&sharp), energy(&soft));
    assert!((e1 - e0).abs() / e0 < 0.03, "{e0} vs {e1}");
    // σ = 20: the centre is well below full and light reaches 50 px out.
    let centre = px(&soft, 100, 100)[0];
    assert!(centre > 60 && centre < 140, "{centre}");
    assert!(px(&soft, 35, 100)[0] > 0);
}

#[test]
fn a_blur_radius_scales_with_the_output() {
    // The same plan at half size: the radius halves with the canvas, so
    // the half-size frame matches the full one's profile at half scale.
    let mut images = Images::default().with(1, solid(40, 40, WHITE));
    let l = with_motion(
        layer(still(1), (40, 40), translate(80.0, 80.0)),
        motion(8.0, 1.0, Direction::Left, 0.0),
    );
    let p = plan(200, 200, vec![l]);
    let full = compositor().render(&p, 200, 200, &mut images);
    let half = compositor().render(&p, 100, 100, &mut images);
    for x in 30..50u32 {
        let a = f64::from(px(&half, x, 50)[0]);
        let b =
            (f64::from(px(&full, 2 * x, 100)[0]) + f64::from(px(&full, 2 * x + 1, 100)[0])) / 2.0;
        assert!((a - b).abs() <= 12.0, "{x}: {a} {b}");
    }
}

// --- Wipe ---

fn wiped(shown: f64, from: Direction) -> Frame {
    let mut images = Images::default().with(1, solid(200, 100, WHITE));
    let l = with_motion(
        layer(still(1), (200, 100), translate(50.0, 50.0)),
        motion(0.0, shown, from, 0.0),
    );
    compositor().render(&plan(300, 200, vec![l]), 300, 200, &mut images)
}

/// The shown fraction FX-030's wipe gives at `u` (0..1 across the layer).
fn wipe_at(shown: f64, u: f64) -> f64 {
    let f = 0.08;
    let edge = shown * (1.0 + f) - f / 2.0;
    ((edge - u) / f + 0.5).clamp(0.0, 1.0)
}

#[test]
fn a_wipe_from_the_left_puts_its_soft_edge_where_the_share_says() {
    let f = wiped(0.5, Direction::Left);
    for x in 50..250u32 {
        let u = (f64::from(x) + 0.5 - 50.0) / 200.0;
        let want = 255.0 * wipe_at(0.5, u);
        let got = f64::from(px(&f, x, 100)[0]);
        assert!((got - want).abs() <= 1.0, "x {x}: {got} vs {want}");
    }
    // The edge's middle is at half the layer; the feather is 8 % (16 px).
    assert_eq!(px(&f, 141, 100), WHITE);
    assert_eq!(px(&f, 159, 100), BG);
    let mid = px(&f, 150, 100)[0];
    assert!(mid.abs_diff(128) <= 10, "{mid}");
    // Rows are alike.
    for y in 50..150 {
        assert_eq!(px(&f, 150, y), px(&f, 150, 100));
    }
}

#[test]
fn wipes_from_every_side() {
    let at = |f: &Frame, x: u32, y: u32| px(f, x, y)[0];
    // From the right, a quarter shown: the right quarter.
    let r = wiped(0.25, Direction::Right);
    assert_eq!(at(&r, 240, 100), 255);
    assert_eq!(at(&r, 60, 100), 0);
    let u = |x: u32| (250.0 - (f64::from(x) + 0.5)) / 200.0;
    for x in [190, 196, 200, 204, 210] {
        let want = 255.0 * wipe_at(0.25, u(x));
        assert!((f64::from(at(&r, x, 100)) - want).abs() <= 1.0, "{x}");
    }
    // From the top (Up) and the bottom (Down), along y over 100 px.
    let up = wiped(0.25, Direction::Up);
    let down = wiped(0.25, Direction::Down);
    for y in 50..150u32 {
        let v = (f64::from(y) + 0.5 - 50.0) / 100.0;
        let want_up = 255.0 * wipe_at(0.25, v);
        let want_down = 255.0 * wipe_at(0.25, 1.0 - v);
        assert!(
            (f64::from(at(&up, 150, y)) - want_up).abs() <= 1.0,
            "up {y}"
        );
        assert!(
            (f64::from(at(&down, 150, y)) - want_down).abs() <= 1.0,
            "down {y}"
        );
    }
}

#[test]
fn a_wipe_starts_with_nothing_and_ends_on_the_still_layer() {
    let nothing = wiped(0.0, Direction::Left);
    assert!(nothing.rgba.chunks(4).all(|p| p == BG));
    // A sliver: only the start side, faintly.
    let sliver = wiped(0.01, Direction::Left);
    assert!(px(&sliver, 50, 100)[0] > 0 && px(&sliver, 249, 100) == BG);
    let mut images = Images::default().with(1, solid(200, 100, WHITE));
    let still = compositor().render(
        &plan(
            300,
            200,
            vec![layer(still(1), (200, 100), translate(50.0, 50.0))],
        ),
        300,
        200,
        &mut images,
    );
    assert_eq!(wiped(1.0, Direction::Left), still);
    // Just below 1, the far edge is still exactly shown.
    let almost = wiped(0.999, Direction::Left);
    assert_eq!(px(&almost, 249, 100)[0], 255);
}

// --- Tint ---

fn tinted(amount: f64, pic: Picture) -> Frame {
    let mut images = Images::default().with(1, pic);
    let l = with_motion(
        layer(still(1), (16, 9), DMat3::IDENTITY),
        motion(0.0, 1.0, Direction::Left, amount),
    );
    compositor().render(&plan(16, 9, vec![l]), 16, 9, &mut images)
}

#[test]
fn tint_endpoints_are_the_layer_and_the_tint_colour() {
    let pic = pattern(16, 9);
    let untouched = tinted(0.0, pic.clone());
    let mut images = Images::default().with(1, pic.clone());
    let plain = compositor().render(
        &plan(16, 9, vec![layer(still(1), (16, 9), DMat3::IDENTITY)]),
        16,
        9,
        &mut images,
    );
    assert_eq!(untouched, plain);
    // Full white tint: every texel white at its own alpha (over black).
    let white = tinted(1.0, pic.clone());
    for (got, src) in white.rgba.chunks(4).zip(pic.rgba.chunks(4)) {
        let a = src[3];
        assert!(
            got[0].abs_diff(a) <= 1 && got[1] == got[0] && got[2] == got[0],
            "{got:?} {src:?}"
        );
    }
    // Half way: each channel half way to white (opaque texels).
    let half = tinted(0.5, pic.clone());
    for (got, src) in half.rgba.chunks(4).zip(pic.rgba.chunks(4)) {
        if src[3] == 255 {
            for c in 0..3 {
                let want = (f64::from(src[c]) + 255.0) / 2.0;
                assert!((f64::from(got[c]) - want).abs() <= 1.5, "{got:?} {src:?}");
            }
        }
    }
}

// --- 3D turns ---

fn halves(w: u32, h: u32) -> Picture {
    let rgba = (0..w * h)
        .flat_map(|i| if i % w < w / 2 { RED } else { BLUE })
        .collect();
    Picture::new(w, h, rgba)
}

#[test]
fn a_turned_layer_lands_on_the_projected_corners() {
    let (cw, ch) = (400u32, 300u32);
    let size = (200.0, 100.0);
    let m = turned(DVec2::new(200.0, 150.0), size, 0.0, 40.0, 1.5 * size.0);
    let mut images = Images::default().with(1, solid(200, 100, RED));
    let f = compositor().render(
        &plan(cw, ch, vec![layer(still(1), (200, 100), m)]),
        cw,
        ch,
        &mut images,
    );
    let corners = [(0.0, 0.0), (200.0, 0.0), (200.0, 100.0), (0.0, 100.0)]
        .map(|(x, y)| project(m, DVec2::new(x, y)));
    // The ink's bounds are the projected quad's.
    let ink = ink_box(&f, BG).unwrap();
    let lo = corners.iter().fold(DVec2::INFINITY, |a, &c| a.min(c));
    let hi = corners.iter().fold(DVec2::NEG_INFINITY, |a, &c| a.max(c));
    assert!(
        (f64::from(ink.0) - lo.x.floor()).abs() <= 1.0,
        "{ink:?} {lo}"
    );
    assert!(
        (f64::from(ink.1) - lo.y.floor()).abs() <= 1.0,
        "{ink:?} {lo}"
    );
    assert!(
        (f64::from(ink.2) - hi.x.ceil()).abs() <= 1.0,
        "{ink:?} {hi}"
    );
    assert!(
        (f64::from(ink.3) - hi.y.ceil()).abs() <= 1.0,
        "{ink:?} {hi}"
    );
    // Points just inside each corner are drawn, just outside are not.
    let centroid = corners.iter().copied().sum::<DVec2>() / 4.0;
    for c in corners {
        let dir = (centroid - c).normalize();
        let inside = c + dir * 3.0;
        let outside = c - dir * 3.0;
        assert_eq!(px(&f, inside.x as u32, inside.y as u32), RED, "{c}");
        assert_eq!(px(&f, outside.x as u32, outside.y as u32), BG, "{c}");
    }
    // Perspective: the right edge turned away is shorter than the left.
    let left = corners[3].y - corners[0].y;
    let right = corners[2].y - corners[1].y;
    assert!(right < left * 0.9, "{left} {right}");
    // Every drawn pixel maps back inside the layer through the inverse.
    let inv = m.inverse();
    for y in 0..ch {
        for x in 0..cw {
            if px(&f, x, y) == BG {
                continue;
            }
            let q = inv * DVec3::new(f64::from(x) + 0.5, f64::from(y) + 0.5, 1.0);
            let (lx, ly) = (q.x / q.z, q.y / q.z);
            assert!(
                q.z > 0.0 && lx > -1.5 && lx < 201.5 && ly > -1.5 && ly < 101.5,
                "{x},{y} → {lx},{ly}"
            );
        }
    }
}

#[test]
fn edge_on_layers_draw_nothing_and_back_faces_show_mirrored() {
    let size = (200.0, 100.0);
    let render = |turn_y: f64| {
        let m = turned(DVec2::new(200.0, 150.0), size, 0.0, turn_y, 1.5 * size.0);
        let mut images = Images::default().with(1, halves(200, 100));
        compositor().render(
            &plan(400, 300, vec![layer(still(1), (200, 100), m)]),
            400,
            300,
            &mut images,
        )
    };
    assert!(render(90.0).rgba.chunks(4).all(|p| p == BG), "edge-on");
    // Turned 150°: the back of the layer, its left half now on the right.
    let back = render(150.0);
    assert_eq!(px(&back, 250, 150), RED);
    assert_eq!(px(&back, 150, 150), BLUE);
    let front = render(30.0);
    assert_eq!(px(&front, 150, 150), RED);
    assert_eq!(px(&front, 250, 150), BLUE);
}

#[test]
fn points_behind_the_eye_are_not_drawn() {
    // The eye close to the layer (0.2 × its width) and a 60° turn: the
    // left part of the layer passes behind the eye.
    let size = (200.0, 100.0);
    let m = turned(DVec2::new(200.0, 150.0), size, 0.0, 60.0, 40.0);
    let mut images = Images::default().with(1, solid(200, 100, RED));
    let f = compositor().render(
        &plan(400, 300, vec![layer(still(1), (200, 100), m)]),
        400,
        300,
        &mut images,
    );
    let inv = m.inverse();
    // The preimage of an output point: in front of the eye and inside the
    // layer (`None` when behind the eye).
    let preimage = |x: f64, y: f64| {
        let q = inv * DVec3::new(x, y, 1.0);
        (q.z > 0.0).then(|| DVec2::new(q.x / q.z, q.y / q.z))
    };
    let inside = |p: DVec2| p.x > -0.01 && p.x < 200.01 && p.y > -0.01 && p.y < 100.01;
    let (mut drawn, mut behind) = (0, 0);
    for y in 0..300u32 {
        for x in 0..400u32 {
            let (cx, cy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            if preimage(cx, cy).is_none() {
                behind += 1;
            }
            if px(&f, x, y) == BG {
                continue;
            }
            drawn += 1;
            // Drawn pixels lie in front of the eye, and the pixel's box
            // reaches the layer.
            let centre = preimage(cx, cy);
            assert!(centre.is_some(), "{x},{y} is behind the eye");
            let touches = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0), (0.5, 0.5)]
                .iter()
                .filter_map(|&(dx, dy)| preimage(f64::from(x) + dx, f64::from(y) + dy))
                .any(inside);
            assert!(touches, "{x},{y}: preimage {centre:?}");
        }
    }
    assert!(drawn > 1000, "{drawn}");
    assert!(
        behind > 1000,
        "the test must reach behind the eye: {behind}"
    );
}

#[test]
fn turned_text_is_drawn_in_perspective() {
    let (cw, ch) = (400u32, 200u32);
    let mut c = compositor();
    let spec = TextSpec {
        size_px: 60.0,
        fill: Fill::Solid([255, 255, 255, 255]),
        ..TextSpec::new("HHHH")
    };
    let (bw, bh) = c.text_renderer().box_size(&spec);
    let (bw, bh) = (f64::from(bw), f64::from(bh));
    // Text layer coordinates have the box centre at the origin.
    let m = turned(DVec2::new(200.0, 100.0), (0.0, 0.0), 0.0, 45.0, 1.5 * bw);
    let mut images = Images::default();
    images.texts.insert(
        clip(1),
        TextLayer {
            spec,
            layer_to_canvas: m,
            anchor: TextAnchor::Center,
            motion: None,
        },
    );
    let text_layer = layer(
        LayerSource::Text {
            data: Arc::new(TextClipData::default()),
            scale_px: 1.0,
        },
        (cw, ch),
        m,
    );
    let f = c.render(&plan(cw, ch, vec![text_layer]), cw, ch, &mut images);
    let ink = ink_box(&f, BG).expect("drawn");
    let corners = [(-bw, -bh), (bw, -bh), (bw, bh), (-bw, bh)]
        .map(|(x, y)| project(m, DVec2::new(x / 2.0, y / 2.0)));
    let lo = corners.iter().fold(DVec2::INFINITY, |a, &c| a.min(c));
    let hi = corners.iter().fold(DVec2::NEG_INFINITY, |a, &c| a.max(c));
    assert!(
        f64::from(ink.0) >= lo.x - 2.0 && f64::from(ink.2) <= hi.x + 2.0,
        "{ink:?} {lo} {hi}"
    );
    assert!(
        f64::from(ink.1) >= lo.y - 2.0 && f64::from(ink.3) <= hi.y + 2.0,
        "{ink:?} {lo} {hi}"
    );
    // Foreshortened: narrower than the flat box.
    assert!(f64::from(ink.2 - ink.0) < bw * 0.85, "{ink:?} {bw}");
    // The far (right) letters are shorter than the near (left) ones.
    let column_height = |x: u32| (0..ch).filter(|&y| px(&f, x, y) != BG).count();
    let (l, r) = (
        (ink.0..ink.0 + 20).map(column_height).max().unwrap(),
        (ink.2 - 20..ink.2).map(column_height).max().unwrap(),
    );
    assert!(r < l, "{l} {r}");
}

// --- Determinism and caching ---

fn busy_plan() -> (FramePlan, Images) {
    let size = (320.0, 180.0);
    let turn = turned(DVec2::new(240.0, 135.0), size, 10.0, 35.0, 1.5 * size.0);
    let images = Images::default()
        .with(1, pattern(320, 180))
        .with(2, pattern(64, 64));
    let layers = vec![
        with_motion(
            layer(still(1), (320, 180), turn),
            motion(6.0, 0.7, Direction::Up, 0.3),
        ),
        with_motion(
            layer(
                still(2),
                (64, 64),
                translate(30.0, 40.0) * DMat3::from_angle(0.3),
            ),
            motion(0.0, 0.6, Direction::Right, 0.6),
        ),
        with_motion(
            layer(
                LayerSource::Generated {
                    kind: freecut_timeline::model::GeneratedKind::Sticker,
                    local_time: Tick::ZERO,
                },
                (100, 60),
                translate(350.0, 180.0),
            ),
            motion(3.0, 1.0, Direction::Left, 0.0),
        ),
    ];
    (plan(480, 270, layers), images)
}

#[test]
fn effects_render_identically_on_any_thread_count() {
    let (p, mut images) = busy_plan();
    let one = compositor()
        .with_threads(1)
        .render(&p, 480, 270, &mut images);
    for threads in [2, 3, 8] {
        let many = compositor()
            .with_threads(threads)
            .render(&p, 480, 270, &mut images);
        assert!(one == many, "{threads} threads differ");
    }
    assert!(ink_box(&one, BG).is_some());
}

#[test]
fn blurred_layers_are_cached_while_their_motion_holds() {
    let (p, mut images) = busy_plan();
    let mut c = compositor();
    let first = c.render(&p, 480, 270, &mut images);
    assert_eq!(c.cached_blurs(), 2);
    let again = c.render(&p, 480, 270, &mut images);
    assert_eq!(c.cached_blurs(), 2, "a still frame reuses its blurs");
    assert!(first == again);
    // A new blur radius is a new entry.
    let mut moved = p.clone();
    let freecut_timeline::plan::PlanNode::Layer(l) = &mut moved.nodes[0];
    l.motion = motion(7.0, 0.7, Direction::Up, 0.3);
    let changed = c.render(&moved, 480, 270, &mut images);
    assert_eq!(c.cached_blurs(), 3);
    assert!(changed != first);
}

// --- Per-letter tint (Shimmer) ---

fn red_spec(text: &str) -> TextSpec {
    TextSpec {
        size_px: 50.0,
        fill: Fill::Solid([255, 0, 0, 255]),
        ..TextSpec::new(text)
    }
}

/// The mean green of the fill in each letter's advance (0: red, 255:
/// white).
fn letter_greens(r: &mut TextRenderer, spec: &TextSpec, raster: &TextRaster) -> Vec<f64> {
    let layout = r.layout(spec);
    layout
        .glyphs
        .iter()
        .map(|g| {
            let s = raster.scale;
            let x0 = (g.origin.0 * s) as i32 + raster.origin.0;
            let x1 = ((g.origin.0 + g.advance) * s) as i32 + raster.origin.0;
            let (mut sum, mut n) = (0.0, 0.0);
            for y in 0..raster.image.height as i32 {
                for x in x0.max(0)..x1.min(raster.image.width as i32) {
                    let i = ((y as u32 * raster.image.width + x as u32) * 4) as usize;
                    let p = &raster.image.rgba[i..i + 4];
                    if p[3] > 200 {
                        sum += f64::from(p[1]) / f64::from(p[3]) * 255.0;
                        n += 1.0;
                    }
                }
            }
            if n > 0.0 { sum / n } else { 0.0 }
        })
        .collect()
}

#[test]
fn the_shimmer_band_travels_across_the_letters() {
    let mut r = TextRenderer::new(FontRegistry::bundled());
    let spec = red_spec("HHHH");
    // TX-056's Shimmer: each unit lit over a fifth of the cycle, peaking
    // at a tenth; the delays spread the units over the cycle.
    let band = UnitChannels::Table(ChannelTable(vec![ChannelCurve {
        channel: GlyphChannel::Tint,
        points: vec![(0.0, 0.0), (0.1, 0.8), (0.2, 0.0), (1.0, 0.0)],
        hold: false,
        idle: None,
        noise: None,
    }]));
    let cycle = Tick::from_raw(1000);
    let at = |t: i64| {
        TextMotion::new(
            AnimUnit::Grapheme,
            UnitOrder::Forward,
            UnitTiming::Loop {
                cycle,
                overlap: PerMille::ZERO,
            },
            Tick::from_raw(t),
            band.clone(),
        )
    };
    for unit in 0..4u32 {
        // The unit's peak: its delay plus a tenth of the cycle.
        let delay = loop_delay(cycle, 4, unit, PerMille::ZERO).raw();
        let raster = r.rasterize_animated(&spec, &at(delay + 100), 1.0);
        let greens = letter_greens(&mut r, &spec, &raster);
        let lit = greens
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0;
        assert_eq!(lit as u32, unit, "{greens:?}");
        // 80 % of the way to white at the peak; the others untouched.
        assert!(
            (greens[unit as usize] - 0.8 * 255.0).abs() < 8.0,
            "{greens:?}"
        );
        for (i, g) in greens.iter().enumerate() {
            if i as u32 != unit {
                assert!(*g < 3.0, "{greens:?}");
            }
        }
    }
    // Between peaks no letter is lit: the band has left unit 3 and not
    // yet wrapped.
    let quiet = r.rasterize_animated(&spec, &at(950), 1.0);
    assert!(
        letter_greens(&mut r, &spec, &quiet)
            .iter()
            .all(|g| *g < 3.0)
    );
}

#[test]
fn tinted_glyphs_change_the_raster_cache_key() {
    let mut r = TextRenderer::new(FontRegistry::bundled());
    let spec = red_spec("Hi");
    let lit = |k: f32| {
        TextMotion::new(
            AnimUnit::Grapheme,
            UnitOrder::Forward,
            UnitTiming::In {
                span: Tick::from_raw(10),
                overlap: PerMille::ONE,
            },
            Tick::ZERO,
            UnitChannels::Table(ChannelTable(vec![ChannelCurve {
                channel: GlyphChannel::Tint,
                points: vec![(0.0, k)],
                hold: false,
                idle: None,
                noise: None,
            }])),
        )
    };
    let a = r.rasterize_animated(&spec, &lit(0.5), 1.0);
    let b = r.rasterize_animated(&spec, &lit(0.6), 1.0);
    assert!(!Arc::ptr_eq(&a, &b));
    let mut blue = lit(0.5);
    blue.tint_color = [0, 0, 255];
    let c = r.rasterize_animated(&spec, &blue, 1.0);
    assert!(!Arc::ptr_eq(&a, &c));
    assert!(Arc::ptr_eq(
        &a,
        &r.rasterize_animated(&spec, &lit(0.5), 1.0)
    ));
}

// --- Typewriter caret and Decode through plans ---

const W: u32 = 480;
const H: u32 = 270;

fn ms(v: i64) -> Tick {
    Tick::from_raw(TICKS_PER_SECOND / 1000 * v)
}

/// A decoding In over the whole slot, per letter (TX-056's Decode).
fn decode_manifest() -> PresetManifest {
    PresetManifest {
        id: PresetId::new("test.in.decode"),
        kind: PresetKind::Entrance,
        units: Some(vec![ModelUnit::Letter]),
        directions: None,
        default_duration: ms(1000),
        easing: Interp::Linear,
        pivot: Pivot::Anchor,
        channels: vec![Channel {
            property: ChannelProp::Decode,
            curve: Curve::Eased,
            amplitude: 1.0,
            unit: ChannelUnit::Ratio,
            scales_with_intensity: false,
        }],
        defaults: ParamDefaults {
            overlap: PerMille::ONE,
            ..ParamDefaults::default()
        },
    }
}

/// A block wipe and a block white flash (whole-layer effects on text).
fn block_manifest(id: &str, property: ChannelProp, amplitude: f64) -> PresetManifest {
    PresetManifest {
        id: PresetId::new(id),
        kind: PresetKind::Entrance,
        units: None,
        directions: Some(vec![Direction::Left, Direction::Right]),
        default_duration: ms(1000),
        easing: Interp::Linear,
        pivot: Pivot::Anchor,
        channels: vec![Channel {
            property,
            curve: Curve::Eased,
            amplitude,
            unit: ChannelUnit::Ratio,
            scales_with_intensity: false,
        }],
        defaults: ParamDefaults::default(),
    }
}

fn table() -> PresetTable {
    let mut all = samples();
    all.push(decode_manifest());
    all.push(block_manifest("test.in.wipe", ChannelProp::Reveal, -1.0));
    all.push(block_manifest("test.in.flash", ChannelProp::Tint, 1.0));
    PresetTable::new(all)
}

fn titled(fps: u32, text: &str) -> (Project, freecut_timeline::ids::ClipId) {
    let mut p = Project::new(W, H, fps);
    let clip = p.add_text(
        TextClipData::default(),
        Tick::ZERO,
        Tick::from_raw(TICKS_PER_SECOND * 3),
    );
    p.set_content(clip, text);
    p.style(clip, TextStyleEdit::Size(RefPx::from_px(160)));
    (p, clip)
}

fn frame_at(c: &mut Compositor, table: &PresetTable, p: &Project, t: Tick) -> Frame {
    let none = BTreeSet::new();
    let plan = c
        .plan(
            p.editor.document(),
            p.seq(),
            t,
            &PlanInputs { offline: &none },
            table,
        )
        .unwrap();
    c.render(&plan, W, H, &mut NoImages)
}

#[test]
fn the_typewriter_caret_option_blinks_after_typing() {
    let (mut p, clip) = titled(30, "HHHHH");
    let mut s = AnimationSpec::new(PresetId::new("fc.in.typewriter"), ms(2000));
    s.params.overlap = PerMille::ZERO;
    s.params.pace = Some(ms(400));
    p.animate(clip, Slot::Entrance, Some(s));
    let table = table();
    let options = TextMotionOptions {
        caret: Some(Caret { blink: ms(200) }),
        ..TextMotionOptions::default()
    };
    let mut with = compositor().with_text_motion_options(options);
    let mut without = compositor();
    let at = |ms_: i64| Tick::from_raw(TICKS_PER_SECOND / 1000 * ms_);
    let mut caret_shown = |p: &Project, t: i64| {
        frame_at(&mut with, &table, p, at(t)) != frame_at(&mut without, &table, p, at(t))
    };
    // While typing (letters at 0, 0.4, …, 1.6 s) the caret is steady.
    for t in [0, 300, 700, 1500] {
        assert!(caret_shown(&p, t), "typing, {t} ms");
    }
    // From the last letter (1.6 s): on for 0.1 s, off for 0.1 s, …
    assert!(caret_shown(&p, 1650));
    assert!(!caret_shown(&p, 1750));
    assert!(caret_shown(&p, 1850));
    assert!(!caret_shown(&p, 1950));
    // The slot is over: no caret.
    assert!(!caret_shown(&p, 2100));
    // Presets that don't type never get one.
    p.animate(
        clip,
        Slot::Entrance,
        Some(AnimationSpec::new(PresetId::new("fc.in.fade"), ms(2000))),
    );
    assert!(!caret_shown(&p, 1000));
}

#[test]
fn the_decode_step_counts_frames_of_the_sequence() {
    let (mut p, clip) = titled(30, "NOHUD");
    p.animate(
        clip,
        Slot::Entrance,
        Some(AnimationSpec::new(
            PresetId::new("test.in.decode"),
            ms(2000),
        )),
    );
    let table = table();
    let frame_ticks = TICKS_PER_SECOND / 30;
    let f = |n: i64| Tick::from_raw(frame_ticks * n);
    let distinct = |c: &mut Compositor, frames: std::ops::Range<i64>| {
        let mut seen: Vec<Frame> = Vec::new();
        for n in frames {
            let fr = frame_at(c, &table, &p, f(n));
            if !seen.contains(&fr) {
                seen.push(fr);
            }
        }
        seen.len()
    };
    // Every frame: eight frames, eight sets of stand-ins (seeded draws may
    // repeat a set, rarely).
    let mut every = compositor().with_text_motion_options(TextMotionOptions {
        decode_step: DecodeStep::Frames(1),
        ..TextMotionOptions::default()
    });
    assert!(distinct(&mut every, 0..8) >= 6);
    // Every four frames: frames 0..4 alike, 4..8 alike.
    let mut four = compositor().with_text_motion_options(TextMotionOptions {
        decode_step: DecodeStep::Frames(4),
        ..TextMotionOptions::default()
    });
    assert_eq!(distinct(&mut four, 0..4), 1);
    assert_eq!(distinct(&mut four, 4..8), 1);
    assert_ne!(
        frame_at(&mut four, &table, &p, f(3)),
        frame_at(&mut four, &table, &p, f(4))
    );
    // A fixed step ignores the frame rate.
    let mut fixed = compositor().with_text_motion_options(TextMotionOptions {
        decode_step: DecodeStep::Fixed(f(2)),
        ..TextMotionOptions::default()
    });
    assert_eq!(distinct(&mut fixed, 0..2), 1);
    assert_eq!(distinct(&mut fixed, 2..4), 1);
}

#[test]
fn block_wipe_and_flash_presets_play_on_text() {
    let (mut p, clip) = titled(30, "HHHHH");
    let table = table();
    let mut c = compositor();
    let still = frame_at(&mut c, &table, &p, ms(1500));
    let ink = ink_box(&still, BG).unwrap();
    // Wipe from the left, half way: the left half of the title shows.
    p.animate(
        clip,
        Slot::Entrance,
        Some(AnimationSpec::new(PresetId::new("test.in.wipe"), ms(1000))),
    );
    let half = frame_at(&mut c, &table, &p, ms(500));
    let shown = ink_box(&half, BG).unwrap();
    assert_eq!(shown.0, ink.0);
    let mid = f64::from(ink.0 + ink.2) / 2.0;
    assert!(
        f64::from(shown.2) < mid + 0.1 * f64::from(ink.2 - ink.0),
        "{shown:?} {ink:?}"
    );
    assert!(
        f64::from(shown.2) > mid - 0.2 * f64::from(ink.2 - ink.0),
        "{shown:?} {ink:?}"
    );
    assert_eq!(frame_at(&mut c, &table, &p, ms(1000)), still);
    // White flash: starts white and ends on the still title.
    p.animate(
        clip,
        Slot::Entrance,
        Some(AnimationSpec::new(PresetId::new("test.in.flash"), ms(1000))),
    );
    let flash = frame_at(&mut c, &table, &p, Tick::ZERO);
    let brightest = flash.rgba.chunks(4).max_by_key(|p| p[1]).unwrap();
    assert!(brightest[1] > 240 && brightest[2] > 240, "{brightest:?}");
    assert_eq!(frame_at(&mut c, &table, &p, ms(1000)), still);
}

// --- Time ---

/// A 1080p picture turned in 3D and blurred, as FX-030's 3D flip and Blur
/// in would draw it. Prints the times; the bound only catches gross
/// regressions (this crate builds at opt-level 2 in debug).
#[test]
fn a_blurred_turned_1080p_layer_renders_within_budget() {
    let (w, h) = (1920u32, 1080u32);
    let size = (1920.0, 1080.0);
    let mut images = Images::default().with(1, pattern(w, h));
    let frame_for = |blur: f64, turn: f64| {
        let m = turned(DVec2::new(960.0, 540.0), size, 0.0, turn, 1.5 * size.0);
        plan(
            w,
            h,
            vec![with_motion(
                layer(still(1), (w, h), m),
                motion(blur, 1.0, Direction::Left, 0.0),
            )],
        )
    };
    let mut c = compositor();
    let mut time = |label: &str, p: &FramePlan, c: &mut Compositor| {
        let start = Instant::now();
        let f = c.render(p, w, h, &mut images);
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        println!("{label}: {ms:.1} ms");
        assert!(ink_box(&f, BG).is_some());
        ms
    };
    let plain = frame_for(0.0, 0.0);
    time("1080p layer, plain", &plain, &mut c);
    let turn = frame_for(0.0, 35.0);
    time("1080p layer, turned 35°", &turn, &mut c);
    let mut worst: f64 = 0.0;
    for (i, blur) in [12.0, 21.6, 40.0].into_iter().enumerate() {
        let p = frame_for(blur, 35.0 - i as f64);
        worst = worst.max(time(
            &format!("1080p layer, turned and blurred {blur} px (uncached)"),
            &p,
            &mut c,
        ));
        time(
            &format!("1080p layer, turned and blurred {blur} px (cached)"),
            &p,
            &mut c,
        );
    }
    assert!(worst < 5000.0, "{worst} ms");
}

#[test]
fn the_typewriter_caret_rides_after_the_letter_just_typed() {
    // The bundled Typewriter steps opacity (`fc.in.typewriter`); the bar
    // follows the typed letter, not the end of the hidden text (TX-056).
    let (mut p, clip) = titled(30, "HHHHH");
    let mut s = AnimationSpec::new(PresetId::new("fc.in.typewriter"), ms(2000));
    s.params.overlap = PerMille::ZERO;
    s.params.pace = Some(ms(400));
    p.animate(clip, Slot::Entrance, Some(s.clone()));
    let table = table();
    let mut c = compositor();
    let without = frame_at(&mut c, &table, &p, ms(100));
    s.params.caret = true;
    p.animate(clip, Slot::Entrance, Some(s));
    let with = frame_at(&mut c, &table, &p, ms(100));
    // The typed letter's ink, and the columns only the caret paints.
    let typed = frame_ink(&without).unwrap();
    let caret: Vec<u32> = (0..W)
        .filter(|&x| (0..H).any(|y| px(&with, x, y) != px(&without, x, y)))
        .collect();
    let (first, last) = (caret[0], *caret.last().unwrap());
    assert!(
        first + 2 >= typed.2 && first <= typed.2 + 12 && last < typed.2 + 30,
        "letter ends at {}, caret at {first}..{last}",
        typed.2
    );
}

fn frame_ink(f: &Frame) -> Option<(u32, u32, u32, u32)> {
    let mut b: Option<(u32, u32, u32, u32)> = None;
    for y in 0..f.height {
        for x in 0..f.width {
            if px(f, x, y) != BG {
                b = Some(match b {
                    None => (x, y, x + 1, y + 1),
                    Some(q) => (q.0.min(x), q.1.min(y), q.2.max(x + 1), q.3.max(y + 1)),
                });
            }
        }
    }
    b
}
