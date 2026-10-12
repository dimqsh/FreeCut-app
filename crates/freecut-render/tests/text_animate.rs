//! Per-unit text animation drawn with the bundled fonts (TX-022 steps 3
//! and 4, TX-023, TX-056): glyphs moved about their own centres, reveals
//! and the boxes that follow them, Decode and the caret, the layer cache,
//! the composition with block motion, determinism across threads, and the
//! time a moving 1080p title takes.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::sync::Arc;
use std::time::Instant;

use common::BG;
use freecut_render::cpu::{Compositor, Frame, LayerImages, Picture, TextAnchor, TextLayer};
use freecut_render::shape::bubble::{BubbleKind, BubbleSpec};
use freecut_render::text::animate::{
    AnimUnit, Caret, ChannelCurve, ChannelTable, Decode, GlyphAnim, GlyphChannel, TextMotion,
    UnitChannels, UnitFn, UnitOrder, UnitSample, UnitTiming,
};
use freecut_render::text::{
    BoxShape, FontRegistry, Glow, GlowDirection, Shadow, Stroke, StrokePosition, TextBox,
    TextBubble, TextRaster, TextRenderer, TextSpec,
};
use freecut_timeline::glam::{DMat3, DVec2};
use freecut_timeline::ids::MediaId;
use freecut_timeline::model::PerMille;
use freecut_timeline::plan::{LayerPlan, LayerSource};
use freecut_timeline::time::Tick;

fn t(v: i64) -> Tick {
    Tick::from_raw(v)
}

fn renderer() -> TextRenderer {
    TextRenderer::new(FontRegistry::bundled())
}

fn spec(text: &str) -> TextSpec {
    TextSpec {
        size_px: 50.0,
        ..TextSpec::new(text)
    }
}

fn curve(channel: GlyphChannel, points: &[(f32, f32)]) -> ChannelCurve {
    ChannelCurve {
        channel,
        points: points.to_vec(),
        hold: false,
        idle: None,
        noise: None,
    }
}

fn table(curves: Vec<ChannelCurve>) -> UnitChannels {
    UnitChannels::Table(ChannelTable(curves))
}

/// Every unit in the same pose (overlap 100 %), at progress `at / 1000`.
fn together(unit: AnimUnit, channels: UnitChannels, at: i64) -> TextMotion {
    TextMotion::new(
        unit,
        UnitOrder::Forward,
        UnitTiming::In {
            span: t(1000),
            overlap: PerMille::ONE,
        },
        t(at),
        channels,
    )
}

/// A constant pose for every unit.
fn posed(unit: AnimUnit, anim: GlyphAnim) -> TextMotion {
    together(
        unit,
        UnitChannels::Callback(UnitFn::new(move |_: &UnitSample| anim)),
        0,
    )
}

/// Letter rise (TX-056): 0.6 em below and transparent → rest.
fn letter_rise() -> UnitChannels {
    table(vec![
        curve(GlyphChannel::OffsetY, &[(0.0, 0.6), (1.0, 0.0)]),
        curve(GlyphChannel::Opacity, &[(0.0, 0.0), (1.0, 1.0)]),
    ])
}

fn typewriter() -> UnitChannels {
    table(vec![ChannelCurve {
        hold: true,
        idle: Some(0.0),
        ..curve(GlyphChannel::Reveal, &[(0.0, 1.0)])
    }])
}

fn sequential(channels: UnitChannels, span: i64, at: i64) -> TextMotion {
    TextMotion::new(
        AnimUnit::Grapheme,
        UnitOrder::Forward,
        UnitTiming::In {
            span: t(span),
            overlap: PerMille::ZERO,
        },
        t(at),
        channels,
    )
}

fn pixel(r: &TextRaster, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * r.image.width + x) * 4) as usize;
    let p = &r.image.rgba[i..i + 4];
    [p[0], p[1], p[2], p[3]]
}

/// The bounds (box layout px at scale 1: x0, y0, x1, y1) of the pixels
/// that pass `pick`.
fn bounds_of(r: &TextRaster, pick: impl Fn([u8; 4]) -> bool) -> Option<(f32, f32, f32, f32)> {
    let mut b: Option<(u32, u32, u32, u32)> = None;
    for y in 0..r.image.height {
        for x in 0..r.image.width {
            if pick(pixel(r, x, y)) {
                b = Some(match b {
                    None => (x, y, x + 1, y + 1),
                    Some(q) => (q.0.min(x), q.1.min(y), q.2.max(x + 1), q.3.max(y + 1)),
                });
            }
        }
    }
    let s = r.scale;
    b.map(|(x0, y0, x1, y1)| {
        (
            (x0 as f32 - r.origin.0 as f32) / s,
            (y0 as f32 - r.origin.1 as f32) / s,
            (x1 as f32 - r.origin.0 as f32) / s,
            (y1 as f32 - r.origin.1 as f32) / s,
        )
    })
}

fn ink(r: &TextRaster) -> (f32, f32, f32, f32) {
    bounds_of(r, |p| p[3] > 0).expect("some ink")
}

fn centre(b: (f32, f32, f32, f32)) -> (f32, f32) {
    ((b.0 + b.2) / 2.0, (b.1 + b.3) / 2.0)
}

fn near(a: f32, b: f32, tol: f32) -> bool {
    (a - b).abs() <= tol
}

// --- Glyph motion about each unit's own centre ---

#[test]
fn a_rise_moves_every_glyph_and_its_outline() {
    let mut r = renderer();
    let s = TextSpec {
        stroke: Some(Stroke {
            color: [255, 0, 0, 255],
            width: 0.08,
            opacity: 1.0,
            position: StrokePosition::Outside,
        }),
        ..spec("Rise")
    };
    let rest = r.rasterize(&s, 1.0);
    let moved = r.rasterize_animated(
        &s,
        &posed(
            AnimUnit::Grapheme,
            GlyphAnim {
                offset: (0.0, 0.6),
                ..GlyphAnim::REST
            },
        ),
        1.0,
    );
    let red = |p: [u8; 4]| p[0] > 200 && p[1] < 60 && p[3] > 200;
    let (a, b) = (
        bounds_of(&rest, red).unwrap(),
        bounds_of(&moved, red).unwrap(),
    );
    // 0.6 em = 30 px down; the outline came along.
    assert!(
        near(b.1 - a.1, 30.0, 1.0) && near(b.3 - a.3, 30.0, 1.0),
        "{a:?} {b:?}"
    );
    assert!(near(b.0, a.0, 1.0) && near(b.2, a.2, 1.0), "{a:?} {b:?}");
    // Animation never reflows: the layout is the rest layout.
    assert_eq!(moved.layout, rest.layout);
    assert_eq!(moved.origin.0, rest.origin.0);
}

#[test]
fn per_letter_rise_staggers_the_letters() {
    let mut r = renderer();
    let s = spec("HHHH");
    // Four windows of 100 ticks; at 150 the second letter is halfway.
    let f = r.rasterize_animated(&s, &sequential(letter_rise(), 400, 150), 1.0);
    let layout = r.layout(&s);
    let cell = |i: usize| {
        let g = &layout.glyphs[i];
        let x0 = g.origin.0;
        bounds_of(&f, |_| true).map(|_| ()).unwrap();
        // Ink rows within this glyph's advance.
        let mut rows = Vec::new();
        for y in 0..f.image.height {
            for x in 0..f.image.width {
                let lx = (x as f32 - f.origin.0 as f32) / f.scale;
                if lx >= x0 + 2.0 && lx < x0 + g.advance - 2.0 && pixel(&f, x, y)[3] > 0 {
                    rows.push(y as f32 - f.origin.1 as f32);
                }
            }
        }
        rows.into_iter().fold(f32::INFINITY, f32::min)
    };
    let rest_top = cell_top_at_rest(&mut r, &s);
    // Letter 0 at rest, letter 1 0.3 em (15 px) lower; letters 2, 3 are
    // transparent (start pose) and draw nothing.
    assert!(near(cell(0), rest_top, 1.0), "{} {rest_top}", cell(0));
    assert!(near(cell(1), rest_top + 15.0, 1.0), "{}", cell(1));
    assert_eq!(cell(2), f32::INFINITY);
    assert_eq!(cell(3), f32::INFINITY);
}

fn cell_top_at_rest(r: &mut TextRenderer, s: &TextSpec) -> f32 {
    ink(&r.rasterize(s, 1.0)).1
}

#[test]
fn scale_and_rotation_turn_each_letter_about_its_own_centre() {
    let mut r = renderer();
    let s = spec("I");
    let rest = ink(&r.rasterize(&s, 1.0));
    let big = ink(&r.rasterize_animated(
        &s,
        &posed(
            AnimUnit::Grapheme,
            GlyphAnim {
                scale: 2.0,
                ..GlyphAnim::REST
            },
        ),
        1.0,
    ));
    assert!(near(centre(big).0, centre(rest).0, 1.0), "{rest:?} {big:?}");
    assert!(near(centre(big).1, centre(rest).1, 1.0), "{rest:?} {big:?}");
    assert!(
        near(big.3 - big.1, 2.0 * (rest.3 - rest.1), 2.0),
        "{rest:?} {big:?}"
    );
    let turned = ink(&r.rasterize_animated(
        &s,
        &posed(
            AnimUnit::Grapheme,
            GlyphAnim {
                rotation: 90.0,
                ..GlyphAnim::REST
            },
        ),
        1.0,
    ));
    // An upright bar becomes a lying one around the same centre.
    assert!(near(centre(turned).0, centre(rest).0, 1.0), "{turned:?}");
    assert!(near(centre(turned).1, centre(rest).1, 1.0), "{turned:?}");
    assert!(
        near(turned.2 - turned.0, rest.3 - rest.1, 2.0),
        "{turned:?}"
    );
    assert!(
        near(turned.3 - turned.1, rest.2 - rest.0, 2.0),
        "{turned:?}"
    );

    // Words turn about the word's centre: "a b" as two words, the second
    // one rotated by 180° stays where it is.
    let two = spec("ab cd");
    let flipped = ink(&r.rasterize_animated(
        &two,
        &posed(
            AnimUnit::Word,
            GlyphAnim {
                rotation: 180.0,
                ..GlyphAnim::REST
            },
        ),
        1.0,
    ));
    let rest2 = ink(&r.rasterize(&two, 1.0));
    assert!(near(flipped.0, rest2.0, 3.0) && near(flipped.2, rest2.2, 3.0));
}

#[test]
fn a_3d_turn_foreshortens_and_edge_on_hides() {
    let mut r = renderer();
    let s = spec("H");
    let rest = ink(&r.rasterize(&s, 1.0));
    let turn = |deg: f32| {
        posed(
            AnimUnit::Grapheme,
            GlyphAnim {
                turn: (0.0, deg),
                ..GlyphAnim::REST
            },
        )
    };
    let half = ink(&r.rasterize_animated(&s, &turn(60.0), 1.0));
    let w = |b: (f32, f32, f32, f32)| b.2 - b.0;
    assert!(
        w(half) < 0.65 * w(rest) && w(half) > 0.35 * w(rest),
        "{rest:?} {half:?}"
    );
    assert!(near(centre(half).1, centre(rest).1, 2.0));
    let edge = r.rasterize_animated(&s, &turn(90.0), 1.0);
    assert!(edge.image.is_empty() || bounds_of(&edge, |p| p[3] > 8).is_none());
    // About the horizontal axis (Flip open's per-unit form).
    let flip = ink(&r.rasterize_animated(
        &s,
        &posed(
            AnimUnit::Grapheme,
            GlyphAnim {
                turn: (60.0, 0.0),
                ..GlyphAnim::REST
            },
        ),
        1.0,
    ));
    // Foreshortened in height, about the letter's centre; the near edge
    // grows in perspective, the far one shrinks.
    let h = |b: (f32, f32, f32, f32)| b.3 - b.1;
    assert!(h(flip) < 0.65 * h(rest), "{rest:?} {flip:?}");
    assert!(
        near(centre(flip).0, centre(rest).0, 2.0),
        "{rest:?} {flip:?}"
    );
}

#[test]
fn blur_and_opacity_apply_per_glyph_with_its_effects() {
    let mut r = renderer();
    let s = TextSpec {
        stroke: Some(Stroke {
            color: [255, 0, 0, 255],
            width: 0.1,
            opacity: 1.0,
            position: StrokePosition::Outside,
        }),
        ..spec("O")
    };
    let rest = r.rasterize(&s, 1.0);
    let faded = r.rasterize_animated(
        &s,
        &posed(
            AnimUnit::Grapheme,
            GlyphAnim {
                opacity: 0.5,
                ..GlyphAnim::REST
            },
        ),
        1.0,
    );
    let max_alpha = |r: &TextRaster| r.image.rgba.chunks(4).map(|p| p[3]).max().unwrap();
    assert_eq!(max_alpha(&rest), 255);
    // The fill and the outline fade together.
    assert!(
        (120..=135).contains(&max_alpha(&faded)),
        "{}",
        max_alpha(&faded)
    );
    let red_alpha = faded
        .image
        .rgba
        .chunks(4)
        .filter(|p| p[0] > p[1] + 40)
        .map(|p| p[3])
        .max()
        .unwrap();
    assert!((120..=135).contains(&red_alpha), "{red_alpha}");

    let blurred = r.rasterize_animated(
        &s,
        &posed(
            AnimUnit::Grapheme,
            GlyphAnim {
                blur: 0.2,
                ..GlyphAnim::REST
            },
        ),
        1.0,
    );
    let (a, b) = (ink(&rest), ink(&blurred));
    assert!(b.0 < a.0 - 3.0 && b.2 > a.2 + 3.0, "{a:?} {b:?}");
}

// --- Reveals and boxes ---

#[test]
fn typewriter_reveals_letters_and_the_per_line_box_follows() {
    let mut r = renderer();
    let s = TextSpec {
        background: Some(TextBox {
            color: [0, 0, 255, 255],
            opacity: 1.0,
            shape: BoxShape::PerLine,
            ..TextBox::default()
        }),
        ..spec("HHHHHHHH")
    };
    let full = r.rasterize(&s, 1.0);
    let half = r.rasterize_animated(&s, &sequential(typewriter(), 800, 350), 1.0);
    let blue = |p: [u8; 4]| p[2] > 200 && p[0] < 60;
    let (bf, bh) = (
        bounds_of(&full, blue).unwrap(),
        bounds_of(&half, blue).unwrap(),
    );
    let layout = r.layout(&s);
    // Four letters typed at 350: the box ends after the fourth.
    let fourth = &layout.glyphs[3];
    assert!(near(bh.0, bf.0, 1.0));
    assert!(bh.2 < bf.2 - 3.0 * fourth.advance, "{bf:?} {bh:?}");
    let white = |p: [u8; 4]| p[0] > 200 && p[1] > 200 && p[2] > 200;
    let text = bounds_of(&half, white).unwrap();
    assert!(text.2 <= fourth.origin.0 + fourth.advance + 1.0, "{text:?}");
    // Nothing typed yet: no box.
    let none = r.rasterize_animated(
        &s,
        &TextMotion {
            at: t(-1),
            ..sequential(typewriter(), 800, 0)
        },
        1.0,
    );
    assert!(bounds_of(&none, blue).is_none());
}

#[test]
fn per_character_boxes_move_with_their_glyph() {
    let mut r = renderer();
    let s = TextSpec {
        background: Some(TextBox {
            color: [0, 0, 255, 255],
            opacity: 1.0,
            shape: BoxShape::PerChar,
            ..TextBox::default()
        }),
        ..spec("AB")
    };
    let blue = |p: [u8; 4]| p[2] > 200 && p[0] < 60;
    let rest = bounds_of(&r.rasterize(&s, 1.0), blue).unwrap();
    let moved = bounds_of(
        &r.rasterize_animated(
            &s,
            &posed(
                AnimUnit::Grapheme,
                GlyphAnim {
                    offset: (0.0, -0.4),
                    ..GlyphAnim::REST
                },
            ),
            1.0,
        ),
        blue,
    )
    .unwrap();
    assert!(near(moved.1, rest.1 - 20.0, 1.0) && near(moved.3, rest.3 - 20.0, 1.0));
}

#[test]
fn the_bubble_is_never_revealed_per_unit() {
    let mut r = renderer();
    let s = TextSpec {
        bubble: Some(TextBubble {
            spec: BubbleSpec {
                kind: BubbleKind::Rounded,
                ..BubbleSpec::default()
            },
            fill: [0, 0, 255, 255],
            opacity: 1.0,
            ..TextBubble::default()
        }),
        ..spec("Hello")
    };
    let blue = |p: [u8; 4]| p[2] > 200 && p[0] < 60;
    let rest = bounds_of(&r.rasterize(&s, 1.0), blue).unwrap();
    let start = r.rasterize_animated(&s, &sequential(typewriter(), 500, 0), 1.0);
    assert_eq!(bounds_of(&start, blue).unwrap(), rest);
}

#[test]
fn a_partly_revealed_word_is_clipped_across_its_box() {
    let mut r = renderer();
    let s = spec("HHHH");
    let rest = ink(&r.rasterize(&s, 1.0));
    let part = ink(&r.rasterize_animated(
        &s,
        &posed(
            AnimUnit::Word,
            GlyphAnim {
                reveal: 0.6,
                ..GlyphAnim::REST
            },
        ),
        1.0,
    ));
    // The word's box is its glyphs' advances; 60 % of it falls inside the
    // third letter.
    let layout = r.layout(&s);
    let last = layout.glyphs.last().unwrap();
    let clip = 0.6 * (last.origin.0 + last.advance);
    assert!(clip > layout.glyphs[2].origin.0 + 5.0);
    assert!(near(part.0, rest.0, 0.5));
    assert!(near(part.2, clip, 1.0), "{part:?} {clip}");
}

#[test]
fn the_caret_rides_after_the_typed_letters() {
    let mut r = renderer();
    let s = spec("HH");
    let motion = TextMotion {
        caret: Some(Caret { blink: t(1000) }),
        ..sequential(typewriter(), 200, 50)
    };
    let f = r.rasterize_animated(&s, &motion, 1.0);
    let layout = r.layout(&s);
    let first = &layout.glyphs[0];
    let b = ink(&f);
    // One letter and the 0.08 em bar after it.
    let caret_right = first.origin.0 + first.advance + 0.08 * 50.0;
    assert!(near(b.2, caret_right, 1.0), "{b:?} {caret_right}");
    assert!(near(b.1, layout.lines[0].top, 1.0) && near(b.3, layout.lines[0].bottom, 1.0));
}

// --- Decode ---

#[test]
fn decode_keeps_the_layout_and_draws_stand_ins_in_place() {
    let mut r = renderer();
    let s = spec("NOHUD");
    let rest = r.rasterize(&s, 1.0);
    let motion = TextMotion {
        decode: Some(Decode {
            seed: 5,
            step: t(2),
        }),
        ..sequential(table(Vec::new()), 100, 0)
    };
    let f = r.rasterize_animated(&s, &motion, 1.0);
    // Glyph positions are unchanged (TX-022: animation never reflows).
    assert_eq!(f.layout, rest.layout);
    assert_ne!(f.image.rgba, rest.image.rgba, "stand-ins are drawn");
    // Each stand-in sits centred in the advance it replaces: the ink spans
    // the same cells.
    let (a, b) = (ink(&rest), ink(&f));
    assert!(near(a.0, b.0, 6.0) && near(a.2, b.2, 6.0), "{a:?} {b:?}");
    // Landed: exactly the rest raster.
    let landed = r.rasterize_animated(
        &s,
        &TextMotion {
            at: t(100),
            ..motion
        },
        1.0,
    );
    assert!(Arc::ptr_eq(&landed, &rest));
}

// --- The layer cache ---

#[test]
fn static_states_reuse_cached_rasters() {
    let mut r = renderer();
    let s = TextSpec {
        glow: Some(Glow {
            color: [255, 200, 0, 255],
            intensity: 0.8,
            radius: 0.3,
            offset: 0.0,
            angle_deg: 0.0,
            direction: GlowDirection::Outward,
        }),
        ..spec("Cache")
    };
    let rest = r.rasterize(&s, 1.0);
    // At rest (after the In) the still raster is returned.
    let after = r.rasterize_animated(&s, &sequential(letter_rise(), 500, 500), 1.0);
    assert!(Arc::ptr_eq(&rest, &after));
    // Typewriter holds a pose for a whole window: frames within it share
    // one raster, and repeating a frame adds nothing to the cache.
    let a = r.rasterize_animated(&s, &sequential(typewriter(), 500, 10), 1.0);
    let n = r.cached_rasters();
    let b = r.rasterize_animated(&s, &sequential(typewriter(), 500, 90), 1.0);
    assert!(Arc::ptr_eq(&a, &b));
    assert_eq!(r.cached_rasters(), n);
    // The next letter is a new state.
    let c = r.rasterize_animated(&s, &sequential(typewriter(), 500, 110), 1.0);
    assert!(!Arc::ptr_eq(&a, &c));
}

// --- Composition with block motion (TX-022) ---

/// Supplies each text layer with extra block motion (folded into
/// `layer_to_canvas`, as the frame plan folds the slot's block offsets
/// into the keyframed transform) and a per-unit motion.
struct Animate {
    block: DMat3,
    motion: Option<TextMotion>,
}

impl LayerImages for Animate {
    fn video(&mut self, _: MediaId, _: Tick) -> Option<Arc<Picture>> {
        None
    }

    fn still(&mut self, _: MediaId) -> Option<Arc<Picture>> {
        None
    }

    fn text(&mut self, layer: &LayerPlan) -> Option<TextLayer> {
        let mut text = TextLayer::from_plan(layer)?;
        text.layer_to_canvas = self.block * text.layer_to_canvas;
        text.motion = self.motion.clone();
        Some(text)
    }
}

fn frame_ink(f: &Frame) -> Option<(u32, u32, u32, u32)> {
    let mut b: Option<(u32, u32, u32, u32)> = None;
    for y in 0..f.height {
        for x in 0..f.width {
            let i = ((y * f.width + x) * 4) as usize;
            if f.rgba[i..i + 4] != BG {
                b = Some(match b {
                    None => (x, y, x + 1, y + 1),
                    Some(q) => (q.0.min(x), q.1.min(y), q.2.max(x + 1), q.3.max(y + 1)),
                });
            }
        }
    }
    b
}

/// The composition-order test of TX-022 (keyframed position, a slide In
/// and a per-letter rise on one clip), at the render boundary: the
/// keyframed position and the slide's block offset are one translation in
/// `layer_to_canvas`, the rise is per-unit motion in the text's own frame.
/// The ink lands at rest + position + slide + rise. The same order through
/// the timeline's frame plan is
/// `animated_frames::composition_keyed_position_then_per_letter_rise`;
/// the AI-008 tracking binding waits for AI-008.
#[test]
fn composition_block_offset_then_per_letter_rise() {
    let (w, h) = (480, 270);
    let plan = text_plan(w, h);
    let mut c = Compositor::new(FontRegistry::bundled());
    let rest = c.render(
        &plan,
        w,
        h,
        &mut Animate {
            block: DMat3::IDENTITY,
            motion: None,
        },
    );
    let keyframed = DVec2::new(30.0, -10.0);
    let slide = DVec2::new(-50.0, 0.0);
    // Every letter halfway through Letter rise: 0.3 em lower, half opaque.
    let rise = together(AnimUnit::Grapheme, letter_rise(), 500);
    let moved = c.render(
        &plan,
        w,
        h,
        &mut Animate {
            block: DMat3::from_translation(keyframed + slide),
            motion: Some(rise),
        },
    );
    let (a, b) = (frame_ink(&rest).unwrap(), frame_ink(&moved).unwrap());
    // 40 px text: 0.3 em = 12 px.
    let dx = b.0 as i64 - a.0 as i64;
    let dy = b.1 as i64 - a.1 as i64;
    assert!((dx - (-20)).abs() <= 1, "{a:?} {b:?}");
    assert!((dy - (-10 + 12)).abs() <= 1, "{a:?} {b:?}");
    assert_eq!(
        (b.2 - b.0, b.3 - b.1),
        (a.2 - a.0, a.3 - a.1),
        "the rise moves, it doesn't stretch"
    );
}

fn text_plan(w: u32, h: u32) -> freecut_timeline::plan::FramePlan {
    use freecut_timeline::model::{RefPx, RichText, TextClipData, TextStyle};
    let data = TextClipData {
        content: RichText::new("Title"),
        style: TextStyle {
            size: RefPx::from_px(160),
            ..TextStyle::default()
        },
        ..TextClipData::default()
    };
    // A ref px is a quarter of a canvas pixel at 480 × 270.
    common::plan(
        w,
        h,
        vec![common::layer(
            LayerSource::Text {
                data: Arc::new(data),
                scale_px: 0.25,
            },
            (w, h),
            DMat3::from_translation(DVec2::new(f64::from(w) / 2.0, f64::from(h) / 2.0)),
        )],
    )
}

// --- Determinism and time ---

/// A title with every effect, its letters rising, turning, spinning,
/// blurring and fading in a seeded random order.
fn busy_title(size: f32) -> TextSpec {
    TextSpec {
        size_px: size,
        bold: true,
        stroke: Some(Stroke {
            color: [20, 20, 20, 255],
            width: 0.06,
            opacity: 1.0,
            position: StrokePosition::Outside,
        }),
        shadow: Some(Shadow {
            color: [0, 0, 0, 255],
            opacity: 0.6,
            blur: 0.1,
            distance: 0.08,
            angle_deg: 45.0,
            include_box: false,
        }),
        glow: Some(Glow {
            color: [255, 180, 40, 255],
            intensity: 0.8,
            radius: 0.25,
            offset: 0.0,
            angle_deg: 0.0,
            direction: GlowDirection::Outward,
        }),
        ..TextSpec::new("FreeCut Title")
    }
}

fn busy_motion(at: i64) -> TextMotion {
    TextMotion::new(
        AnimUnit::Grapheme,
        UnitOrder::Random { seed: 7 },
        UnitTiming::In {
            span: t(800),
            overlap: PerMille::new(600),
        },
        t(at),
        table(vec![
            curve(GlyphChannel::OffsetY, &[(0.0, 0.6), (1.0, 0.0)]),
            curve(GlyphChannel::Rotation, &[(0.0, 45.0), (1.0, 0.0)]),
            curve(GlyphChannel::TurnY, &[(0.0, 50.0), (1.0, 0.0)]),
            curve(GlyphChannel::Blur, &[(0.0, 0.1), (0.5, 0.0)]),
            curve(GlyphChannel::Opacity, &[(0.0, 0.0), (0.4, 1.0)]),
        ]),
    )
}

#[test]
fn animated_frames_are_identical_across_threads() {
    let frames = || {
        let mut r = renderer();
        let s = busy_title(60.0);
        (0..=8)
            .map(|k| {
                r.rasterize_animated(&s, &busy_motion(k * 100), 1.5)
                    .image
                    .rgba
                    .clone()
            })
            .collect::<Vec<_>>()
    };
    let a = std::thread::spawn(frames);
    let b = std::thread::spawn(frames);
    let (a, b) = (a.join().unwrap(), b.join().unwrap());
    assert_eq!(a.len(), 9);
    for (i, (x, y)) in a.iter().zip(&b).enumerate() {
        assert!(x == y, "frame {i} differs");
    }
    // And the motion really moves: neighbouring frames differ.
    assert_ne!(a[2], a[3]);
}

/// The S2 acceptance target's render half: a 1080p title with outline,
/// shadow and glow animating per letter, through the compositor at the
/// output size. Prints the times; the bound only catches gross
/// regressions (debug builds of this crate run at opt-level 2).
#[test]
fn a_1080p_title_animating_per_letter_renders_within_budget() {
    let (w, h) = (1920, 1080);
    let plan = text_plan(w, h);
    let layer = |motion: Option<TextMotion>| TextLayer {
        spec: busy_title(160.0),
        layer_to_canvas: DMat3::from_translation(DVec2::new(960.0, 540.0)),
        anchor: TextAnchor::Center,
        motion,
    };
    let mut c = Compositor::new(FontRegistry::bundled());
    // The still title, rendered fresh each time (a new compositor, so no
    // raster cache), as the baseline.
    let mut still = Vec::new();
    for _ in 0..3 {
        let mut fresh = Compositor::new(FontRegistry::bundled());
        let start = Instant::now();
        fresh.render(&plan, w, h, &mut Replace(layer(None)));
        still.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    // 24 frames over the slot (30 fps for 0.8 s of span), the cache warm
    // from the frame before as in playback.
    let mut run = |motion: &dyn Fn(i64) -> TextMotion| -> (f64, f64) {
        let mut times = Vec::new();
        for k in 0..=24 {
            let at = k * 1000 / 24;
            let start = Instant::now();
            let frame = c.render(&plan, w, h, &mut Replace(layer(Some(motion(at)))));
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            assert_eq!((frame.width, frame.height), (w, h));
        }
        let warm = &times[1..];
        let mean = warm.iter().sum::<f64>() / warm.len() as f64;
        (mean, warm.iter().copied().fold(0.0, f64::max))
    };
    // TX-056's Letter rise In (0.8 s, overlap 60 %): offset and fade.
    let (rise_mean, rise_max) = run(&|at| TextMotion {
        timing: UnitTiming::In {
            span: t(1000),
            overlap: PerMille::new(600),
        },
        ..sequential(letter_rise(), 1000, at)
    });
    // Every channel at once: rise, spin, 3D turn, blur, fade, random order.
    let (busy_mean, busy_max) = run(&busy_motion);
    eprintln!(
        "1080p title (outline + shadow + glow), per frame: still, uncached {:.1} ms; \
         letter rise mean {rise_mean:.1} ms, max {rise_max:.1} ms; \
         rise + spin + turn + blur + fade mean {busy_mean:.1} ms, max {busy_max:.1} ms",
        still.iter().copied().fold(f64::INFINITY, f64::min),
    );
    assert!(rise_mean < 250.0, "letter rise: mean {rise_mean:.1} ms");
    assert!(busy_mean < 400.0, "busy: mean {busy_mean:.1} ms");
}

/// Replaces every text layer with a fixed one.
struct Replace(TextLayer);

impl LayerImages for Replace {
    fn video(&mut self, _: MediaId, _: Tick) -> Option<Arc<Picture>> {
        None
    }

    fn still(&mut self, _: MediaId) -> Option<Arc<Picture>> {
        None
    }

    fn text(&mut self, _: &LayerPlan) -> Option<TextLayer> {
        Some(self.0.clone())
    }
}

// --- Review fixes: units in the cache key, the opacity-stepped typewriter,
// Decode's fallback, the ligature policy, overlapping opacity, per-glyph
// hue and glow ---

/// The bundled Typewriter (`fc.in.typewriter`): an opacity step, so a
/// letter is invisible until its window starts (its reveal stays whole).
fn stepped_typewriter() -> UnitChannels {
    table(vec![ChannelCurve {
        hold: true,
        idle: Some(0.0),
        ..curve(GlyphChannel::Opacity, &[(0.0, 1.0)])
    }])
}

#[test]
fn rasters_are_cached_per_unit_kind() {
    // Every unit at 30 % scale: letters shrink about their own centres,
    // words about theirs, with identical per-glyph poses.
    let s = spec("Hello wide world");
    let pop = |unit| {
        together(
            unit,
            table(vec![curve(GlyphChannel::Scale, &[(0.0, 0.0), (1.0, 1.0)])]),
            300,
        )
    };
    let mut r = renderer();
    let letters = r.rasterize_animated(&s, &pop(AnimUnit::Grapheme), 1.0);
    let words = r.rasterize_animated(&s, &pop(AnimUnit::Word), 1.0);
    let fresh = renderer().rasterize_animated(&s, &pop(AnimUnit::Word), 1.0);
    assert_ne!(letters.image.rgba, words.image.rgba);
    assert_eq!(
        words.image.rgba, fresh.image.rgba,
        "not the letters' raster"
    );
}

#[test]
fn the_caret_follows_letters_that_appear_by_opacity() {
    let mut r = renderer();
    let s = spec("HHHHH");
    let layout = r.layout(&s);
    let motion = TextMotion {
        caret: Some(Caret { blink: t(1000) }),
        ..sequential(stepped_typewriter(), 500, 50)
    };
    let state = motion.evaluate(&layout, &s.text);
    let first = &layout.glyphs[0];
    let caret = state.caret.unwrap();
    assert!(
        near(caret.x, first.origin.0 + first.advance, 0.01),
        "{caret:?}"
    );
    // Drawn there: one letter and the bar after it.
    let b = ink(&r.rasterize_animated(&s, &motion, 1.0));
    assert!(
        near(b.2, first.origin.0 + first.advance + 0.08 * 50.0, 1.0),
        "{b:?}"
    );
}

#[test]
fn boxes_cover_only_letters_already_visible() {
    let blue = |p: [u8; 4]| p[2] > 200 && p[0] < 60;
    let boxed = |shape| TextSpec {
        background: Some(TextBox {
            color: [0, 0, 255, 255],
            opacity: 1.0,
            shape,
            ..TextBox::default()
        }),
        ..spec("HHHHHHHH")
    };
    let mut r = renderer();
    let layout = r.layout(&boxed(BoxShape::PerLine));
    let first = &layout.glyphs[0];
    // One letter typed (of eight, 100 ticks each).
    let one = sequential(stepped_typewriter(), 800, 50);
    for shape in [BoxShape::PerLine, BoxShape::Block, BoxShape::PerChar] {
        let s = boxed(shape);
        let full = bounds_of(&r.rasterize(&s, 1.0), blue).unwrap();
        let typed = bounds_of(&r.rasterize_animated(&s, &one, 1.0), blue).unwrap();
        assert!(
            typed.2 < first.origin.0 + first.advance + 0.5 * 50.0 && typed.2 < full.2 - 100.0,
            "{shape:?}: {typed:?} of {full:?}"
        );
    }
    // Per-character boxes fade with their letter.
    let s = boxed(BoxShape::PerChar);
    let half = r.rasterize_animated(
        &s,
        &posed(
            AnimUnit::Grapheme,
            GlyphAnim {
                opacity: 0.5,
                ..GlyphAnim::REST
            },
        ),
        1.0,
    );
    let corner = (half.origin.0 as f32 + 1.0, half.origin.1 as f32 + 1.0);
    let p = pixel(&half, corner.0 as u32, corner.1 as u32);
    assert!((100..=155).contains(&p[3]), "{p:?}");
}

/// A Decode request as `fc.in.decode` makes it: no channel but Decode.
fn decode_only(at: i64) -> TextMotion {
    TextMotion {
        decode: Some(Decode {
            seed: 7,
            step: t(10),
        }),
        ..TextMotion::new(
            AnimUnit::Grapheme,
            UnitOrder::Forward,
            UnitTiming::In {
                span: t(1000),
                overlap: PerMille::new(600),
            },
            t(at),
            table(Vec::new()),
        )
    }
}

#[test]
fn decode_plays_as_letter_fade_where_nothing_can_stand_in() {
    let mut r = renderer();
    // Joined letters move as words; a single distinct letter has nothing
    // to swap with.
    for text in ["مرحبا بكم", "AAAA"] {
        let s = spec(text);
        let layout = r.layout(&s);
        let early = decode_only(100).evaluate(&layout, text);
        assert!(!early.is_rest(), "{text}");
        assert!(early.glyphs.iter().all(|g| g.stand_in.is_none()));
        // The first unit fading in, the later ones not started.
        let opacity: Vec<f32> = early.glyphs.iter().map(|g| g.anim.opacity).collect();
        assert!(
            opacity.iter().any(|&o| o > 0.0 && o < 1.0),
            "{text}: {opacity:?}"
        );
        assert!(opacity.contains(&0.0), "{text}: {opacity:?}");
        assert!(
            decode_only(1000).evaluate(&layout, text).is_rest(),
            "{text}"
        );
    }
    // Letters that can stand in for each other still do, fully opaque.
    let s = spec("ABCD");
    let state = decode_only(100).evaluate(&r.layout(&s), "ABCD");
    assert!(state.glyphs.iter().any(|g| g.stand_in.is_some()));
    assert!(state.glyphs.iter().all(|g| g.anim.opacity == 1.0));
}

#[test]
fn letters_animate_one_glyph_each_under_the_ligature_policy() {
    use freecut_render::text::animate::UnitMap;
    let mut r = renderer();
    let text = "office fluff";
    let letters = text.chars().filter(|c| !c.is_whitespace()).count();
    let inked = |s: &TextSpec, r: &mut TextRenderer| {
        let layout = r.layout(s);
        let map = UnitMap::new(&layout, &s.text, AnimUnit::Grapheme);
        let empty: Vec<u32> = (0..map.count)
            .filter(|&u| map.glyphs_of(u).all(|i| map.blank[i]))
            .collect();
        (layout.glyphs.len(), map.count, empty)
    };
    // The bundled sans forms ffi and fl.
    let ligated = spec(text);
    let (glyphs, units, empty) = inked(&ligated, &mut r);
    assert!(glyphs < letters + 1, "{glyphs} glyphs");
    // A letter inside another's glyph takes no time of its own.
    assert!(empty.is_empty(), "units without a glyph: {empty:?}");
    assert_eq!(units as usize, glyphs - 1);
    // Off while letters move one by one: one glyph per letter.
    let apart = TextSpec {
        ligatures: false,
        ..spec(text)
    };
    let (glyphs, units, empty) = inked(&apart, &mut r);
    assert_eq!((glyphs, units as usize), (letters + 1, letters));
    assert!(empty.is_empty());
    // And with letter spacing (CSS's rule).
    let spaced = TextSpec {
        letter_spacing: 0.1,
        ..spec(text)
    };
    assert_eq!(r.layout(&spaced).glyphs.len(), letters + 1);
}

#[test]
fn a_faint_letter_passing_over_an_opaque_one_leaves_it_opaque() {
    let mut r = renderer();
    let s = spec("HH");
    let layout = r.layout(&s);
    let step = (layout.glyphs[1].origin.0 - layout.glyphs[0].origin.0) / 50.0;
    let over = UnitFn::new(move |u: &UnitSample| {
        if u.unit == 1 {
            GlyphAnim {
                offset: (-step, 0.0),
                opacity: 0.05,
                ..GlyphAnim::REST
            }
        } else {
            GlyphAnim::REST
        }
    });
    let f = r.rasterize_animated(
        &s,
        &together(AnimUnit::Grapheme, UnitChannels::Callback(over), 0),
        1.0,
    );
    let max = f.image.rgba.chunks(4).map(|p| p[3]).max().unwrap();
    assert!(max >= 254, "max alpha {max}");
    // Two faint letters over each other: more than one, less than opaque.
    let faint = posed(
        AnimUnit::Grapheme,
        GlyphAnim {
            opacity: 0.5,
            ..GlyphAnim::REST
        },
    );
    let one = r.rasterize_animated(&spec("H"), &faint, 1.0);
    let one_max = one.image.rgba.chunks(4).map(|p| p[3]).max().unwrap();
    assert!((120..=135).contains(&one_max), "{one_max}");
}

#[test]
fn a_per_glyph_hue_turn_turns_that_glyphs_outline_too() {
    let mut r = renderer();
    // A white fill has no hue to turn: only the red outline shows the turn.
    let s = TextSpec {
        fill: freecut_render::text::Fill::Solid([255, 255, 255, 255]),
        stroke: Some(Stroke {
            color: [255, 0, 0, 255],
            width: 0.08,
            opacity: 1.0,
            position: StrokePosition::Outside,
        }),
        ..spec("HH")
    };
    let split = r.layout(&s).glyphs[1].origin.0;
    let second = UnitFn::new(|u: &UnitSample| GlyphAnim {
        hue: if u.unit == 1 { 120.0 } else { 0.0 },
        ..GlyphAnim::REST
    });
    let f = r.rasterize_animated(
        &s,
        &together(AnimUnit::Grapheme, UnitChannels::Callback(second), 0),
        1.0,
    );
    let red = |p: [u8; 4]| p[0] > 200 && p[1] < 60 && p[2] < 60 && p[3] > 200;
    let green = |p: [u8; 4]| p[1] > 200 && p[0] < 60 && p[2] < 60 && p[3] > 200;
    let (r0, g1) = (bounds_of(&f, red).unwrap(), bounds_of(&f, green).unwrap());
    assert!(
        r0.2 <= split + 1.0,
        "red outline only round the first: {r0:?}"
    );
    assert!(
        g1.0 >= split - 1.0,
        "green outline only round the second: {g1:?}"
    );
}

#[test]
fn hue_turns_and_glow_scales_per_glyph() {
    let mut r = renderer();
    let s = TextSpec {
        fill: freecut_render::text::Fill::Solid([255, 0, 0, 255]),
        ..spec("HH")
    };
    let layout = r.layout(&s);
    let split = layout.glyphs[1].origin.0;
    let second = |hue: f32, glow: f32| {
        UnitFn::new(move |u: &UnitSample| {
            if u.unit == 1 {
                GlyphAnim {
                    hue,
                    glow,
                    ..GlyphAnim::REST
                }
            } else {
                GlyphAnim::REST
            }
        })
    };
    // The second letter turns a third of the wheel: red to green.
    let f = r.rasterize_animated(
        &s,
        &together(
            AnimUnit::Grapheme,
            UnitChannels::Callback(second(120.0, 1.0)),
            0,
        ),
        1.0,
    );
    let red = |p: [u8; 4]| p[0] > 200 && p[1] < 60 && p[3] > 200;
    let green = |p: [u8; 4]| p[1] > 200 && p[0] < 60 && p[3] > 200;
    let (r0, g1) = (bounds_of(&f, red).unwrap(), bounds_of(&f, green).unwrap());
    assert!(r0.2 <= split + 1.0 && g1.0 >= split - 1.0, "{r0:?} {g1:?}");
    // The glow around the second letter only: off at 0, as set at 1.
    let glowing = TextSpec {
        glow: Some(Glow {
            color: [0, 0, 255, 255],
            intensity: 1.0,
            radius: 0.5,
            ..Glow::default()
        }),
        ..s.clone()
    };
    let lit = |p: [u8; 4]| p[2] > 40 && p[0] < 40;
    let first_off = UnitFn::new(|u: &UnitSample| GlyphAnim {
        glow: if u.unit == 0 { 0.0 } else { 1.0 },
        ..GlyphAnim::REST
    });
    let f = r.rasterize_animated(
        &glowing,
        &together(AnimUnit::Grapheme, UnitChannels::Callback(first_off), 0),
        1.0,
    );
    let rest = r.rasterize(&glowing, 1.0);
    let (glow, all) = (bounds_of(&f, lit).unwrap(), bounds_of(&rest, lit).unwrap());
    assert!(
        glow.0 > all.0 + 10.0,
        "no glow left of the first letter: {glow:?} {all:?}"
    );
    assert!(
        near(glow.2, all.2, 1.0),
        "the second keeps its glow: {glow:?} {all:?}"
    );
}
