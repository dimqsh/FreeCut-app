//! Text clips end to end: a document edited through the timeline's editor
//! (insert a text clip, set its content, style, layout and transform), its
//! frame plan, and the CPU compositor's pixels (TX-001 … TX-005, TR-001,
//! TR-003, TR-015).
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use common::BG;
use common::project::{Project, centre, ink_box};
use freecut_render::cpu::{Compositor, Frame, NoImages};
use freecut_render::text::{FontRegistry, TextRenderer, TextSpec};
use freecut_timeline::edit::{TextLayoutEdit, TextStyleEdit, TransformEdit};
use freecut_timeline::ids::ClipId;
use freecut_timeline::model::{
    Angle, CanvasPos, Fill, PerMille, RefPx, Rgba8, Scale, TextClipData,
};
use freecut_timeline::plan::LayerSource;
use freecut_timeline::time::Tick;

/// 480 × 270: a ref px is a quarter of a canvas pixel.
const W: u32 = 480;
const H: u32 = 270;

fn secs(s: i64) -> Tick {
    Tick::from_secs(s).unwrap()
}

/// A project with one text clip showing `text` at 240 ref px (60 canvas
/// px) over [0 s, 3 s).
fn project_with(text: &str) -> (Project, ClipId) {
    let mut p = Project::new(W, H, 30);
    let clip = p.add_text(TextClipData::default(), Tick::ZERO, secs(3));
    p.set_content(clip, text);
    p.style(clip, TextStyleEdit::Size(RefPx::from_px(240)));
    (p, clip)
}

fn render(p: &Project) -> Frame {
    let plan = p.plan_at(secs(1));
    Compositor::new(FontRegistry::bundled()).render(&plan, W, H, &mut NoImages)
}

/// The text box size in canvas px, as the renderer lays it out.
fn box_size(p: &Project) -> (f64, f64) {
    let plan = p.plan_at(secs(1));
    let layer = plan.nodes[0].layer().unwrap();
    let LayerSource::Text { data, scale_px } = &layer.source else {
        panic!("not a text layer");
    };
    let layout =
        TextRenderer::new(FontRegistry::bundled()).layout(&TextSpec::from_clip(data, *scale_px));
    (f64::from(layout.size.0), f64::from(layout.size.1))
}

fn max_red(frame: &Frame) -> u8 {
    frame.rgba.chunks(4).map(|p| p[0]).max().unwrap()
}

#[test]
fn a_new_text_clip_is_centred_on_the_canvas() {
    let (p, _) = project_with("HH");
    let frame = render(&p);
    let ink = ink_box(&frame, BG).expect("the text is drawn");
    let (cx, cy) = centre(ink);
    // "HH" has equal side bearings: its ink is centred on the box.
    assert!((cx - 240.0).abs() <= 1.5, "ink {ink:?}");
    // The box (not the ink) is centred: the ink lies inside it.
    let (bw, bh) = box_size(&p);
    assert!(
        f64::from(ink.0) >= 240.0 - bw / 2.0 - 1.0,
        "{ink:?} in {bw}x{bh}"
    );
    assert!(
        f64::from(ink.2) <= 240.0 + bw / 2.0 + 1.0,
        "{ink:?} in {bw}x{bh}"
    );
    assert!(
        f64::from(ink.1) >= 135.0 - bh / 2.0 - 1.0,
        "{ink:?} in {bw}x{bh}"
    );
    assert!(
        f64::from(ink.3) <= 135.0 + bh / 2.0 + 1.0,
        "{ink:?} in {bw}x{bh}"
    );
    assert!((cy - 135.0).abs() <= bh / 4.0, "ink {ink:?}");
    // White, fully covered inside the strokes.
    assert!(frame.rgba.chunks(4).any(|px| px == [255, 255, 255, 255]));
}

#[test]
fn the_text_is_shown_only_during_the_clip() {
    let (p, _) = project_with("HH");
    let mut comp = Compositor::new(FontRegistry::bundled());
    let after = comp.render(&p.plan_at(secs(4)), W, H, &mut NoImages);
    assert!(ink_box(&after, BG).is_none());
}

#[test]
fn position_moves_the_text_exactly() {
    let (mut p, clip) = project_with("Move me");
    let before = render(&p);
    // +10 % of the width, −20 % of the height: +48 px, −54 px.
    p.transform(
        clip,
        TransformEdit::Position(CanvasPos::new(100_000, -200_000)),
    );
    let after = render(&p);
    let (a, b) = (ink_box(&before, BG).unwrap(), ink_box(&after, BG).unwrap());
    assert_eq!(
        (b.0, b.1, b.2, b.3),
        (a.0 + 48, a.1 - 54, a.2 + 48, a.3 - 54)
    );
    // Whole-pixel moves resample nothing: the same pixels, moved.
    for y in a.1..=a.3 {
        for x in a.0..=a.2 {
            assert_eq!(
                before.pixel(x, y),
                after.pixel(x + 48, y - 54),
                "({x}, {y})"
            );
        }
    }
}

#[test]
fn rotation_turns_the_text_about_its_centre() {
    let (mut p, clip) = project_with("IIIIIIIIII");
    let flat = ink_box(&render(&p), BG).unwrap();
    p.transform(clip, TransformEdit::Rotation(Angle::from_degrees(90)));
    let turned = ink_box(&render(&p), BG).unwrap();
    let size = |b: (u32, u32, u32, u32)| (f64::from(b.2 - b.0 + 1), f64::from(b.3 - b.1 + 1));
    let ((fw, fh), (tw, th)) = (size(flat), size(turned));
    assert!(fw > 2.0 * fh, "flat {flat:?}");
    // A quarter turn swaps the extents (±2 px of resampling).
    assert!(
        (tw - fh).abs() <= 2.0 && (th - fw).abs() <= 2.0,
        "{flat:?} → {turned:?}"
    );
    // The pivot is the box centre, which stays at the canvas centre; a
    // clockwise turn moves the ink's left edge (offset from the centre by
    // the vertical ink offset) predictably, so compare centres loosely.
    let (fc, tc) = (centre(flat), centre(turned));
    let (dx, dy) = (fc.0 - 240.0, fc.1 - 135.0);
    // Clockwise with y down: (dx, dy) → (−dy, dx).
    assert!((tc.0 - (240.0 - dy)).abs() <= 2.0, "{fc:?} → {tc:?}");
    assert!((tc.1 - (135.0 + dx)).abs() <= 2.0, "{fc:?} → {tc:?}");
}

#[test]
fn opacity_scales_the_text() {
    let (mut p, clip) = project_with("HH");
    assert_eq!(max_red(&render(&p)), 255);
    p.transform(clip, TransformEdit::Opacity(PerMille::new(500)));
    let faded = render(&p);
    // Fully covered white at half opacity over black.
    assert_eq!(max_red(&faded), 128);
    assert!(faded.rgba.chunks(4).all(|px| px[3] == 255));
}

#[test]
fn a_fixed_box_width_wraps_the_lines() {
    let (mut p, clip) = project_with("wrap these words please");
    p.style(clip, TextStyleEdit::Size(RefPx::from_px(120))); // 30 px
    let one_line = ink_box(&render(&p), BG).unwrap();
    assert!(one_line.2 - one_line.0 > 200, "{one_line:?}");
    // 600 ref px = 150 canvas px.
    p.layout(clip, TextLayoutEdit::BoxWidth(Some(RefPx::from_px(600))));
    let wrapped = ink_box(&render(&p), BG).unwrap();
    let width = wrapped.2 - wrapped.0 + 1;
    assert!(width <= 152, "wrapped ink {wrapped:?}");
    let (h1, h2) = (one_line.3 - one_line.1 + 1, wrapped.3 - wrapped.1 + 1);
    assert!(h2 >= 2 * h1, "one line {h1} px, wrapped {h2} px");
    // Centred lines in a centred box.
    assert!((centre(wrapped).0 - 240.0).abs() <= 3.0, "{wrapped:?}");
    // Wider line spacing spreads the same lines further apart.
    // (1.5×: the lines still fit on the canvas.)
    p.layout(clip, TextLayoutEdit::LineSpacing(PerMille::new(1500)));
    let spaced = ink_box(&render(&p), BG).unwrap();
    assert!(
        spaced.3 - spaced.1 > wrapped.3 - wrapped.1 + 30,
        "{spaced:?}"
    );
}

#[test]
fn clip_scale_enlarges_without_changing_line_breaks() {
    let (mut p, clip) = project_with("wrap these words please");
    p.style(clip, TextStyleEdit::Size(RefPx::from_px(80))); // 20 px
    p.layout(clip, TextLayoutEdit::BoxWidth(Some(RefPx::from_px(400))));
    let small = ink_box(&render(&p), BG).unwrap();
    p.transform(clip, TransformEdit::Scale(Scale::from_percent(200)));
    let big = ink_box(&render(&p), BG).unwrap();
    let ratio = |a: u32, b: u32, c: u32, d: u32| f64::from(d - c + 1) / f64::from(b - a + 1);
    let wr = ratio(small.0, small.2, big.0, big.2);
    let hr = ratio(small.1, small.3, big.1, big.3);
    assert!(
        (1.9..=2.1).contains(&wr),
        "width ×{wr}: {small:?} → {big:?}"
    );
    assert!(
        (1.9..=2.1).contains(&hr),
        "height ×{hr}: {small:?} → {big:?}"
    );
    let (cs, cb) = (centre(small), centre(big));
    assert!((cb.0 - 240.0).abs() <= 2.0 * (cs.0 - 240.0).abs() + 2.0);
}

#[test]
fn style_edits_change_colour_and_size() {
    let (mut p, clip) = project_with("HH");
    let normal = ink_box(&render(&p), BG).unwrap();
    p.style(
        clip,
        TextStyleEdit::Fill(Fill::Solid(Rgba8::new(255, 0, 0, 255))),
    );
    let red = render(&p);
    assert!(red.rgba.chunks(4).any(|px| px == [255, 0, 0, 255]));
    assert!(red.rgba.chunks(4).all(|px| px[1] == 0 && px[2] == 0));
    p.style(clip, TextStyleEdit::Size(RefPx::from_px(480)));
    let big = ink_box(&render(&p), BG).unwrap();
    let ratio = f64::from(big.2 - big.0 + 1) / f64::from(normal.2 - normal.0 + 1);
    assert!((1.9..=2.1).contains(&ratio), "×{ratio}");
}

#[test]
fn flips_mirror_the_text() {
    let (mut p, clip) = project_with("Fb");
    let plain = render(&p);
    p.transform(clip, TransformEdit::FlipH(true));
    let flipped = render(&p);
    assert_ne!(plain, flipped);
    let (a, b) = (ink_box(&plain, BG).unwrap(), ink_box(&flipped, BG).unwrap());
    // Mirrored about the box centre (the canvas centre).
    assert!(
        (f64::from(a.0) - (480.0 - f64::from(b.2) - 1.0)).abs() <= 1.0,
        "{a:?} {b:?}"
    );
}

#[test]
fn empty_text_draws_nothing() {
    let (mut p, clip) = project_with("HH");
    p.set_content(clip, "");
    assert!(ink_box(&render(&p), BG).is_none());
}

#[test]
fn text_is_rasterized_at_the_preview_size() {
    let (p, _) = project_with("HH");
    let plan = p.plan_at(secs(1));
    let mut comp = Compositor::new(FontRegistry::bundled());
    let full = comp.render(&plan, W, H, &mut NoImages);
    let half = comp.render(&plan, W / 2, H / 2, &mut NoImages);
    let (f, h) = (ink_box(&full, BG).unwrap(), ink_box(&half, BG).unwrap());
    // Same place, half the size, still crisp (fully covered pixels exist).
    assert!(
        (f64::from(h.0) - f64::from(f.0) / 2.0).abs() <= 1.0,
        "{f:?} {h:?}"
    );
    assert!(
        (f64::from(h.2) - f64::from(f.2) / 2.0).abs() <= 1.0,
        "{f:?} {h:?}"
    );
    assert!(half.rgba.chunks(4).any(|px| px == [255, 255, 255, 255]));
}
