//! Round-2 text styles end to end (C46): documents edited through the
//! timeline's editor with `SetTextStyle` / `SetTextLayout` requests, their
//! frame plans, and the CPU compositor's pixels: one test per layer kind
//! (outline, shadow, glow, background, bubble, curve), fill, case, letter
//! spacing, decoration, justify and the vertical anchor (TX-005 … TX-016,
//! TX-054, TX-055); and the export path drawing exactly the preview's bytes
//! for styled titles.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use common::BG;
use common::project::{Project, centre, ink_box};
use freecut_render::cpu::{Compositor, Frame, NoImages};
use freecut_render::export::ExportRenderer;
use freecut_render::text::FontRegistry;
use freecut_timeline::edit::{TextLayoutEdit, TextStyleEdit, TransformEdit};
use freecut_timeline::export::ExportClock;
use freecut_timeline::ids::ClipId;
use freecut_timeline::model::{
    Angle, BoxShape, BubbleKind, CanvasPos, CaseTransform, DeciDegrees, Fill, FillExtent,
    GlowDirection, GradientStops, HAlign, LinearGradient, PerMille, RefPx, Rgba8, Scale,
    SignedPerMille, StrokePosition, TailTip, TextClipData, TextLayerKind, VAnchor,
};
use freecut_timeline::time::{FrameRate, Tick, TimeRange};

/// 480 × 270: a ref px is a quarter of a canvas pixel.
const W: u32 = 480;
const H: u32 = 270;
const RED: Rgba8 = Rgba8::new(255, 0, 0, 255);
const GREEN: Rgba8 = Rgba8::new(0, 255, 0, 255);
const BLUE: Rgba8 = Rgba8::new(0, 0, 255, 255);

fn secs(s: i64) -> Tick {
    Tick::from_secs(s).unwrap()
}

/// A project with one white text clip showing `text` at 240 ref px (60
/// canvas px) over [0 s, 3 s), centred.
fn project_with(text: &str) -> (Project, ClipId) {
    let mut p = Project::new(W, H, 30);
    let clip = p.add_text(TextClipData::default(), Tick::ZERO, secs(3));
    p.set_content(clip, text);
    p.style(clip, TextStyleEdit::Size(RefPx::from_px(240)));
    (p, clip)
}

fn layer(p: &mut Project, clip: ClipId, layer: TextLayerKind, on: bool) {
    p.style(clip, TextStyleEdit::SetLayer { layer, on });
}

fn render(p: &Project) -> Frame {
    Compositor::new(FontRegistry::bundled()).render(&p.plan_at(secs(1)), W, H, &mut NoImages)
}

/// Pixels `(x, y)` of `frame` whose colour passes `pick`.
fn pixels(frame: &Frame, pick: impl Fn([u8; 4]) -> bool) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for y in 0..frame.height {
        for x in 0..frame.width {
            if frame.pixel(x, y).is_some_and(&pick) {
                out.push((x, y));
            }
        }
    }
    out
}

fn mean(points: &[(u32, u32)]) -> (f64, f64) {
    let n = points.len() as f64;
    let (sx, sy) = points.iter().fold((0.0, 0.0), |(sx, sy), &(x, y)| {
        (sx + f64::from(x), sy + f64::from(y))
    });
    (sx / n, sy / n)
}

fn reddish(px: [u8; 4]) -> bool {
    px[0] > 200 && px[1] < 60 && px[2] < 60
}

fn greenish(px: [u8; 4]) -> bool {
    px[1] > 100 && px[0] < 40 && px[2] < 40
}

fn bluish(px: [u8; 4]) -> bool {
    px[2] > 60 && i32::from(px[2]) > i32::from(px[0]) + 40
}

fn white(px: [u8; 4]) -> bool {
    px[0] > 250 && px[1] > 250 && px[2] > 250
}

fn width(b: (u32, u32, u32, u32)) -> u32 {
    b.2 - b.0 + 1
}

fn height(b: (u32, u32, u32, u32)) -> u32 {
    b.3 - b.1 + 1
}

#[test]
fn the_outline_draws_round_the_letters_and_switching_it_off_restores_them() {
    let (mut p, clip) = project_with("HI");
    let plain = render(&p);
    assert!(pixels(&plain, reddish).is_empty());
    layer(&mut p, clip, TextLayerKind::Outline, true);
    p.style(clip, TextStyleEdit::OutlineColor(RED));
    p.style(clip, TextStyleEdit::OutlineWidth(PerMille::new(150))); // 9 px
    let outlined = render(&p);
    let red = pixels(&outlined, reddish);
    assert!(red.len() > 300, "{} red pixels", red.len());
    // Outside: the fill keeps its white core and the ink grows by about
    // the width on every side.
    assert!(!pixels(&outlined, white).is_empty());
    let (a, b) = (
        ink_box(&plain, BG).unwrap(),
        ink_box(&outlined, BG).unwrap(),
    );
    assert!(a.0 - b.0 >= 7 && b.2 - a.2 >= 7, "{a:?} → {b:?}");
    // Inside: over the fill and clipped to it, so the ink keeps its size.
    p.style(clip, TextStyleEdit::OutlinePosition(StrokePosition::Inside));
    let inside = render(&p);
    let c = ink_box(&inside, BG).unwrap();
    assert!(
        a.0.abs_diff(c.0) <= 1 && a.2.abs_diff(c.2) <= 1,
        "{a:?} → {c:?}"
    );
    assert!(!pixels(&inside, reddish).is_empty());
    // Off keeps the values but draws exactly the plain text.
    layer(&mut p, clip, TextLayerKind::Outline, false);
    assert!(render(&p) == plain);
}

#[test]
fn the_shadow_falls_down_and_to_the_right_at_45_degrees() {
    let (mut p, clip) = project_with("HH");
    let ink = pixels(&render(&p), white);
    layer(&mut p, clip, TextLayerKind::Shadow, true);
    p.style(clip, TextStyleEdit::ShadowColor(GREEN));
    p.style(clip, TextStyleEdit::ShadowOpacity(PerMille::ONE));
    p.style(clip, TextStyleEdit::ShadowBlur(PerMille::ZERO));
    p.style(clip, TextStyleEdit::ShadowDistance(PerMille::new(250))); // 15 px
    let frame = render(&p);
    let shadow = pixels(&frame, greenish);
    assert!(shadow.len() > 200, "{} shadow pixels", shadow.len());
    // The shadow is the letters moved 15 px at 45°: ~10.6 px each way
    // (the letters hide part of it, which pulls its mean away further).
    let (ix, iy) = mean(&ink);
    let (sx, sy) = mean(&shadow);
    assert!(
        sx - ix > 5.0 && sy - iy > 5.0,
        "ink ({ix}, {iy}) shadow ({sx}, {sy})"
    );
    // Pointing it up-left (225°) mirrors the offset.
    p.style(
        clip,
        TextStyleEdit::ShadowAngle(DeciDegrees::from_degrees(225)),
    );
    let (ux, uy) = mean(&pixels(&render(&p), greenish));
    assert!(
        ix - ux > 5.0 && iy - uy > 5.0,
        "ink ({ix}, {iy}) shadow ({ux}, {uy})"
    );
}

#[test]
fn an_outward_glow_adds_light_round_the_letters_and_an_inward_one_stays_inside() {
    let (mut p, clip) = project_with("HH");
    let plain = render(&p);
    let ink = ink_box(&plain, BG).unwrap();
    layer(&mut p, clip, TextLayerKind::Glow, true);
    p.style(clip, TextStyleEdit::GlowColor(BLUE));
    p.style(clip, TextStyleEdit::GlowIntensity(PerMille::ONE));
    p.style(clip, TextStyleEdit::GlowRadius(PerMille::new(300))); // 18 px
    let glowing = render(&p);
    let outside = |frame: &Frame| {
        pixels(frame, bluish)
            .into_iter()
            .filter(|&(x, y)| x < ink.0 || x > ink.2 || y < ink.1 || y > ink.3)
            .count()
    };
    assert!(outside(&glowing) > 200, "{} glow pixels", outside(&glowing));
    // The canvas stays opaque: light was added, nothing was punched out.
    assert!(glowing.rgba.chunks(4).all(|px| px[3] == 255));
    // Inward: clipped to the letters, so nothing lights up outside them.
    p.style(clip, TextStyleEdit::GlowDirection(GlowDirection::Inward));
    let inward = render(&p);
    assert_eq!(outside(&inward), 0);
    assert!(inward != plain, "the inward glow tints the letters");
    let b = ink_box(&inward, BG).unwrap();
    assert!(
        b.0.abs_diff(ink.0) <= 1 && b.2.abs_diff(ink.2) <= 1,
        "{ink:?} → {b:?}"
    );
}

#[test]
fn the_background_box_surrounds_the_text_with_its_padding() {
    let (mut p, clip) = project_with("HH");
    let ink = ink_box(&render(&p), BG).unwrap();
    layer(&mut p, clip, TextLayerKind::Background, true);
    p.style(clip, TextStyleEdit::BackgroundColor(RED));
    p.style(clip, TextStyleEdit::BackgroundOpacity(PerMille::ONE));
    p.style(clip, TextStyleEdit::BackgroundRoundness(PerMille::ZERO));
    let frame = render(&p);
    let red = pixels(&frame, reddish);
    let boxed = ink_box(&frame, BG).unwrap();
    // Padding 25 % × 15 % of 60 px: 15 px and 9 px beyond the text box.
    assert!(
        ink.0 - boxed.0 >= 14 && boxed.2 - ink.2 >= 14,
        "{ink:?} → {boxed:?}"
    );
    // A sharp opaque rectangle: every pixel of it that isn't a letter is
    // red, so red covers most of the box.
    let area = (width(boxed) * height(boxed)) as usize;
    assert!(red.len() * 2 > area, "{} red of {area}", red.len());
    assert_eq!(
        frame.pixel(boxed.0 + 1, boxed.1 + 1),
        Some([255, 0, 0, 255])
    );
    // Per line in a lower-half band is shorter than the block.
    p.style(clip, TextStyleEdit::BackgroundShape(BoxShape::PerLine));
    p.style(
        clip,
        TextStyleEdit::BackgroundHeight(freecut_timeline::model::BoxHeight::LowerHalf),
    );
    let band = render(&p);
    let band_red = pixels(&band, reddish);
    assert!(!band_red.is_empty());
    let top = band_red.iter().map(|&(_, y)| y).min().unwrap();
    assert!(
        top > ink.1 + 10,
        "the band starts below the cap height: {top} vs {ink:?}"
    );
}

#[test]
fn the_bubble_and_its_tail_are_drawn_and_replace_the_background() {
    let (mut p, clip) = project_with("Hi");
    layer(&mut p, clip, TextLayerKind::Bubble, true);
    p.style(clip, TextStyleEdit::BubbleColor(RED));
    // Without the dark border the whole tail, tip included, is red.
    p.style(clip, TextStyleEdit::BubbleBorder(false));
    // The tip 50 px left of and 115 px below the box centre (canvas px).
    p.style(
        clip,
        TextStyleEdit::BubbleTailTip(TailTip {
            x: RefPx::from_px(-200),
            y: RefPx::from_px(460),
        }),
    );
    let frame = render(&p);
    let red = pixels(&frame, reddish);
    assert!(red.len() > 2000, "{} red pixels", red.len());
    // The white text sits on the red body.
    assert!(!pixels(&frame, white).is_empty());
    let lowest = |red: &[(u32, u32)]| red.iter().map(|&(_, y)| y).max().unwrap();
    let with_tail = lowest(&red);
    assert!(with_tail >= 240, "the tail reaches down to {with_tail}");
    // The tail points left of centre.
    let tail: Vec<_> = red.iter().copied().filter(|&(_, y)| y > 225).collect();
    assert!(mean(&tail).0 < 240.0, "{:?}", mean(&tail));
    // Without the tail the body ends well above the tip.
    p.style(clip, TextStyleEdit::BubbleTail(false));
    let no_tail = lowest(&pixels(&render(&p), reddish));
    assert!(no_tail + 40 < with_tail, "{no_tail} vs {with_tail}");
    // Every kind draws (Shout's burst included).
    for kind in [
        BubbleKind::Oval,
        BubbleKind::Shout,
        BubbleKind::Thought,
        BubbleKind::Ticket,
    ] {
        p.style(clip, TextStyleEdit::BubbleKind(kind));
        assert!(pixels(&render(&p), reddish).len() > 2000, "{kind:?}");
    }
    // Background on switches the bubble off (TX-054) and draws the box.
    p.style(clip, TextStyleEdit::BubbleKind(BubbleKind::Rounded));
    let bubble = render(&p);
    layer(&mut p, clip, TextLayerKind::Background, true);
    let boxed = render(&p);
    assert!(pixels(&boxed, reddish).is_empty(), "the red bubble is gone");
    assert!(boxed != bubble);
    // And Bubble on again gives back the same bubble, box off.
    layer(&mut p, clip, TextLayerKind::Bubble, true);
    assert!(render(&p) == bubble);
}

#[test]
fn a_curve_bends_the_line_into_an_arch_or_a_bowl() {
    let (mut p, clip) = project_with("CURVED");
    let flat = ink_box(&render(&p), BG).unwrap();
    layer(&mut p, clip, TextLayerKind::Curve, true);
    // On at 0 is still straight.
    assert_eq!(ink_box(&render(&p), BG).unwrap(), flat);
    p.style(clip, TextStyleEdit::Curve(SignedPerMille::new(400)));
    let arch_frame = render(&p);
    let arch = ink_box(&arch_frame, BG).unwrap();
    assert!(height(arch) > height(flat) + 15, "{flat:?} → {arch:?}");
    // An arch: the ends drop, so the left- and rightmost ink is low.
    let ink = pixels(&arch_frame, |px| px != BG);
    let ends: Vec<_> = ink
        .iter()
        .copied()
        .filter(|&(x, _)| x < arch.0 + 10 || x > arch.2 - 10)
        .collect();
    let middle: Vec<_> = ink
        .iter()
        .copied()
        .filter(|&(x, _)| x.abs_diff(240) < 10)
        .collect();
    assert!(mean(&ends).1 > mean(&middle).1 + 10.0);
    // A bowl: the ends rise.
    p.style(clip, TextStyleEdit::Curve(SignedPerMille::new(-400)));
    let bowl_frame = render(&p);
    let bowl = ink_box(&bowl_frame, BG).unwrap();
    let ink = pixels(&bowl_frame, |px| px != BG);
    let ends: Vec<_> = ink
        .iter()
        .copied()
        .filter(|&(x, _)| x < bowl.0 + 10 || x > bowl.2 - 10)
        .collect();
    let middle: Vec<_> = ink
        .iter()
        .copied()
        .filter(|&(x, _)| x.abs_diff(240) < 10)
        .collect();
    assert!(mean(&ends).1 + 10.0 < mean(&middle).1);
    // Off: straight again, the amount kept.
    layer(&mut p, clip, TextLayerKind::Curve, false);
    assert_eq!(ink_box(&render(&p), BG).unwrap(), flat);
}

#[test]
fn a_linear_gradient_runs_from_its_first_stop_to_its_last() {
    let (mut p, clip) = project_with("HHHHHH");
    p.style(
        clip,
        TextStyleEdit::Fill(Fill::LinearGradient(LinearGradient {
            angle: DeciDegrees::ZERO,
            stops: GradientStops::two(RED, BLUE),
            extent: FillExtent::Block,
        })),
    );
    let frame = render(&p);
    let ink = ink_box(&frame, BG).unwrap();
    let solid = |x: u32| {
        (ink.1..=ink.3)
            .filter_map(|y| frame.pixel(x, y))
            .find(|px| u16::from(px[0]) + u16::from(px[2]) > 250)
    };
    // "H" stems at both ends: red on the left, blue on the right.
    let (left, right) = (solid(ink.0 + 2).unwrap(), solid(ink.2 - 2).unwrap());
    assert!(left[0] > 200 && left[2] < 60, "{left:?}");
    assert!(right[2] > 200 && right[0] < 60, "{right:?}");
    // FillKind edits from the stops round trip through the renderer too.
    p.style(
        clip,
        TextStyleEdit::FillAngle(DeciDegrees::from_degrees(180)),
    );
    let flipped = render(&p);
    let px = (ink.1..=ink.3)
        .filter_map(|y| flipped.pixel(ink.0 + 2, y))
        .find(|px| u16::from(px[0]) + u16::from(px[2]) > 250)
        .unwrap();
    assert!(px[2] > 200 && px[0] < 60, "{px:?}");
}

#[test]
fn the_case_transform_draws_what_typing_it_would() {
    let (mut p, clip) = project_with("Mixed case, straße");
    p.style(clip, TextStyleEdit::Case(CaseTransform::Upper));
    let upper = render(&p);
    let (typed, _) = project_with("MIXED CASE, STRASSE");
    assert!(upper == render(&typed));
    p.style(clip, TextStyleEdit::Case(CaseTransform::Lower));
    let (typed, _) = project_with("mixed case, straße");
    assert!(render(&p) == render(&typed));
    // The stored content is untouched: As typed shows it again.
    p.style(clip, TextStyleEdit::Case(CaseTransform::AsTyped));
    let (typed, _) = project_with("Mixed case, straße");
    assert!(render(&p) == render(&typed));
}

#[test]
fn letter_spacing_widens_and_narrows_the_line() {
    let (mut p, clip) = project_with("HHHH");
    let plain = width(ink_box(&render(&p), BG).unwrap());
    // +50 % of 60 px after each of the three inner gaps: +90 px.
    p.style(clip, TextStyleEdit::LetterSpacing(SignedPerMille::new(500)));
    let wide = width(ink_box(&render(&p), BG).unwrap());
    assert!(wide.abs_diff(plain + 90) <= 2, "{plain} → {wide}");
    p.style(
        clip,
        TextStyleEdit::LetterSpacing(SignedPerMille::new(-100)),
    );
    let narrow = width(ink_box(&render(&p), BG).unwrap());
    assert!(narrow.abs_diff(plain - 18) <= 2, "{plain} → {narrow}");
}

#[test]
fn underline_and_strike_add_lines_in_the_fill_colour() {
    let (mut p, clip) = project_with("HH");
    let plain = render(&p);
    let ink = ink_box(&plain, BG).unwrap();
    p.style(clip, TextStyleEdit::Underline(true));
    let under = render(&p);
    let u = ink_box(&under, BG).unwrap();
    assert!(
        u.3 > ink.3,
        "the underline lies below the baseline: {ink:?} → {u:?}"
    );
    let row = u.3 - 1;
    let lit = (u.0..=u.2)
        .filter(|&x| under.pixel(x, row).is_some_and(white))
        .count();
    assert!(lit as u32 * 10 > width(u) * 8, "{lit} of {}", width(u));
    p.style(clip, TextStyleEdit::Underline(false));
    p.style(clip, TextStyleEdit::Strike(true));
    let strike = render(&p);
    assert_eq!(ink_box(&strike, BG).unwrap().3, ink.3);
    // The bar across the middle fills the gap between the letters.
    let between = |frame: &Frame| {
        pixels(frame, white)
            .into_iter()
            .filter(|&(x, _)| x.abs_diff(240) <= 1)
            .count()
    };
    assert!(between(&strike) > between(&plain));
}

#[test]
fn justified_lines_span_the_fixed_box() {
    let (mut p, clip) = project_with("aa bb cc dd ee ff gg hh ii jj kk ll");
    p.style(clip, TextStyleEdit::Size(RefPx::from_px(120))); // 30 px
    // 1200 ref px = 300 canvas px, from x = 90 to 390.
    p.layout(clip, TextLayoutEdit::BoxWidth(Some(RefPx::from_px(1200))));
    p.layout(clip, TextLayoutEdit::Align(HAlign::Left));
    let first_line = |frame: &Frame| {
        let ink = ink_box(frame, BG).unwrap();
        let rows = ink.1..ink.1 + 15;
        let xs: Vec<u32> = pixels(frame, |px| px != BG)
            .into_iter()
            .filter(|(_, y)| rows.contains(y))
            .map(|(x, _)| x)
            .collect();
        (*xs.iter().min().unwrap(), *xs.iter().max().unwrap())
    };
    let left = first_line(&render(&p));
    p.layout(clip, TextLayoutEdit::Justify(true));
    let justified = first_line(&render(&p));
    assert!(left.0.abs_diff(90) <= 4, "{left:?}");
    assert_eq!(justified.0, left.0, "both start at the box's left edge");
    assert!(
        justified.1 >= 384,
        "the first line reaches the right edge: {justified:?}"
    );
    assert!(left.1 + 5 < justified.1, "{left:?} vs {justified:?}");
    // Align switches justify off again.
    p.layout(clip, TextLayoutEdit::Align(HAlign::Left));
    assert_eq!(first_line(&render(&p)), left);
}

#[test]
fn the_vertical_anchor_keeps_its_edge_put_as_lines_are_added() {
    let edges = |v: VAnchor, text: &str| {
        let (mut p, clip) = project_with(text);
        if v != VAnchor::default() {
            p.layout(clip, TextLayoutEdit::VAnchor(v));
        }
        ink_box(&render(&p), BG).unwrap()
    };
    // Top: the box's top edge at the canvas centre, text hanging below.
    let one = edges(VAnchor::Top, "H");
    let two = edges(VAnchor::Top, "H\nH");
    assert!(one.1 >= 135 && one.1 < 135 + 30, "{one:?}");
    assert!(
        one.1.abs_diff(two.1) <= 1,
        "the top stays put: {one:?} {two:?}"
    );
    assert!(two.3 > one.3 + 40);
    // Bottom: the box's bottom edge at the centre, text standing on it.
    let one = edges(VAnchor::Bottom, "H");
    let two = edges(VAnchor::Bottom, "H\nH");
    assert!(one.3 <= 135 && one.3 + 30 > 135, "{one:?}");
    // (±1 px: the raster lands on whole pixels, and the box bottom is a
    // different fraction of a pixel for one line and for two.)
    assert!(
        one.3.abs_diff(two.3) <= 1,
        "the bottom stays put: {one:?} {two:?}"
    );
    assert!(two.1 + 40 < one.1);
    // Middle (the default) straddles the centre and grows both ways.
    let one = edges(VAnchor::Middle, "H");
    let two = edges(VAnchor::Middle, "H\nH");
    assert!(one.1 < 135 && one.3 > 135, "{one:?}");
    assert!(two.1 < one.1 && two.3 > one.3);
    // Horizontally every anchor is the box's centre.
    assert!((centre(one).0 - 240.0).abs() <= 1.5, "{one:?}");
}

#[test]
fn rotation_turns_a_top_anchored_title_about_its_top_edge() {
    let (mut p, clip) = project_with("IIIIIIII");
    p.layout(clip, TextLayoutEdit::VAnchor(VAnchor::Top));
    p.transform(clip, TransformEdit::Rotation(Angle::from_degrees(180)));
    let ink = ink_box(&render(&p), BG).unwrap();
    // Half a turn about the top edge's middle puts the text above it.
    assert!(ink.3 <= 136 && ink.1 < 135 - 30, "{ink:?}");
    assert!((centre(ink).0 - 240.0).abs() <= 2.0, "{ink:?}");
}

/// A project with styled titles one after another, each using several
/// layers, placed and turned: [0 s, 1 s) bubble with tail, gradient,
/// outline, shadow, upper case, bottom anchor; [1 s, 2 s) curved with an
/// inward-and-outward glow and underline; [2 s, 3 s) per-character boxes,
/// letter spacing, justified wrapped lines, top anchor.
fn styled_titles() -> Project {
    let mut p = Project::new(W, H, 30);
    let a = p.add_text(TextClipData::default(), Tick::ZERO, secs(1));
    p.set_content(a, "Speech bubble");
    p.style(a, TextStyleEdit::Size(RefPx::from_px(160)));
    p.style(
        a,
        TextStyleEdit::Fill(Fill::LinearGradient(LinearGradient {
            angle: DeciDegrees::from_degrees(30),
            stops: GradientStops::two(Rgba8::new(255, 200, 0, 255), Rgba8::new(255, 0, 120, 255)),
            extent: FillExtent::Line,
        })),
    );
    p.style(a, TextStyleEdit::Case(CaseTransform::Upper));
    layer(&mut p, a, TextLayerKind::Outline, true);
    layer(&mut p, a, TextLayerKind::Shadow, true);
    layer(&mut p, a, TextLayerKind::Bubble, true);
    p.style(
        a,
        TextStyleEdit::BubbleTailTip(TailTip {
            x: RefPx::from_px(300),
            y: RefPx::from_px(320),
        }),
    );
    p.style(a, TextStyleEdit::BubbleTailBend(SignedPerMille::new(-400)));
    p.layout(a, TextLayoutEdit::VAnchor(VAnchor::Bottom));
    p.transform(a, TransformEdit::Rotation(Angle::from_degrees(-7)));
    p.transform(a, TransformEdit::Position(CanvasPos::new(-50_000, 100_000)));

    let b = p.add_text(TextClipData::default(), secs(1), secs(1));
    p.set_content(b, "On an arc");
    p.style(b, TextStyleEdit::Size(RefPx::from_px(200)));
    p.style(b, TextStyleEdit::Underline(true));
    layer(&mut p, b, TextLayerKind::Glow, true);
    p.style(b, TextStyleEdit::GlowColor(Rgba8::new(0, 200, 255, 255)));
    p.style(b, TextStyleEdit::GlowDirection(GlowDirection::Both));
    layer(&mut p, b, TextLayerKind::Curve, true);
    p.style(b, TextStyleEdit::Curve(SignedPerMille::new(350)));
    p.transform(b, TransformEdit::Scale(Scale::from_percent(130)));

    let c = p.add_text(TextClipData::default(), secs(2), secs(1));
    p.set_content(c, "boxes for every single letter here");
    p.style(c, TextStyleEdit::Size(RefPx::from_px(120)));
    p.style(c, TextStyleEdit::Italic(true));
    p.style(c, TextStyleEdit::LetterSpacing(SignedPerMille::new(80)));
    layer(&mut p, c, TextLayerKind::Background, true);
    p.style(c, TextStyleEdit::BackgroundShape(BoxShape::PerChar));
    p.style(
        c,
        TextStyleEdit::BackgroundColor(Rgba8::new(30, 90, 200, 255)),
    );
    p.style(c, TextStyleEdit::BackgroundBorder(true));
    p.layout(c, TextLayoutEdit::BoxWidth(Some(RefPx::from_px(1400))));
    p.layout(c, TextLayoutEdit::Justify(true));
    p.layout(c, TextLayoutEdit::VAnchor(VAnchor::Top));
    p.transform(c, TransformEdit::Position(CanvasPos::new(0, -300_000)));
    p
}

fn clock(fps: u32) -> ExportClock {
    let range = TimeRange::new(Tick::ZERO, secs(3)).unwrap();
    ExportClock::new(range, FrameRate::new(fps, 1).unwrap()).unwrap()
}

#[test]
fn export_frames_of_styled_titles_are_the_preview_bytes() {
    let p = styled_titles();
    for output in [(W, H), (960, 540), (240, 135)] {
        let clock = clock(30);
        let compositor = Compositor::new(FontRegistry::bundled());
        let mut export =
            ExportRenderer::new(p.doc(), p.seq(), clock, output, compositor, NoImages).unwrap();
        let mut preview = Compositor::new(FontRegistry::bundled());
        for k in [5, 29, 30, 45, 60, 89] {
            let exported = export.render_frame(k).unwrap().clone();
            let plan = p.plan_at(clock.frame_time(k).unwrap());
            let shown = preview.render(&plan, output.0, output.1, &mut NoImages);
            assert!(ink_box(&shown, BG).is_some(), "frame {k} shows a title");
            assert!(
                exported == shown,
                "frame {k} at {output:?} differs from the preview"
            );
        }
    }
}

#[test]
fn styled_titles_render_the_same_bytes_on_any_thread_count_and_cache_state() {
    let p = styled_titles();
    let plans = [15, 45, 75].map(|f| p.plan_at(Tick::from_raw(f * secs(1).raw() / 30)));
    let mut one = Compositor::new(FontRegistry::bundled()).with_threads(1);
    let mut many = Compositor::new(FontRegistry::bundled()).with_threads(4);
    for plan in &plans {
        let a = one.render(plan, W, H, &mut NoImages);
        let b = many.render(plan, W, H, &mut NoImages);
        assert!(a == b, "frame at {:?}", plan.time);
        // A second render comes from the raster cache: still the same.
        assert!(one.render(plan, W, H, &mut NoImages) == a);
    }
    // Rendered on another thread, from scratch.
    let first = one.render(&plans[0], W, H, &mut NoImages);
    let plan = plans[0].clone();
    let other = std::thread::spawn(move || {
        Compositor::new(FontRegistry::bundled()).render(&plan, W, H, &mut NoImages)
    })
    .join()
    .unwrap();
    assert!(first == other);
}
