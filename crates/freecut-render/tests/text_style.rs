//! Styled text on the CPU path (TX-008 … TX-016, C46): outline bands,
//! shadow, glow, boxes, decorations, spacing, justify, curve, gradients,
//! per-glyph layout, the premultiplied text layer in the compositor and
//! determinism across threads. Bundled fonts only.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::sync::Arc;

use common::{Images, clip, plan};
use freecut_render::cpu::{Compositor, TextAnchor, TextLayer};
use freecut_render::shape::Point;
use freecut_render::shape::bubble::{BubbleKind, Tail};
use freecut_render::text::{
    Border, BoxShape, Decoration, Fill, FillExtent, FontRegistry, Glow, GlowDirection,
    GradientStop, LinearGradient, Shadow, Stroke, StrokePosition, TextAlign, TextBox, TextBubble,
    TextRaster, TextRenderer, TextSpec,
};
use freecut_timeline::glam::{DMat3, DVec2};
use freecut_timeline::model::TextClipData;
use freecut_timeline::plan::LayerSource;

fn renderer() -> TextRenderer {
    TextRenderer::new(FontRegistry::bundled())
}

/// "I" at 100 px: a 9 px stem.
fn stem(spec: TextSpec) -> TextSpec {
    TextSpec {
        size_px: 100.0,
        ..TextSpec {
            text: "I".into(),
            ..spec
        }
    }
}

fn px(r: &TextRaster, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * r.image.width + x) * 4) as usize;
    r.image.rgba[i..i + 4].try_into().unwrap()
}

/// The image row half way up the cap height of line 0.
fn mid_row(r: &TextRaster) -> u32 {
    let baseline = r.layout.lines[0].baseline * r.scale;
    (baseline.round() as i32 + r.origin.1 - 35) as u32
}

const RED: [u8; 4] = [255, 0, 0, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

#[test]
fn outline_bands_have_their_width_on_the_right_side_of_the_edge() {
    let mut r = renderer();
    // 4 % of 100 px: a 4 px band at each edge of the stem.
    let outlined = |r: &mut TextRenderer, position| {
        r.rasterize(
            &stem(TextSpec {
                stroke: Some(Stroke {
                    color: RED,
                    width: 0.04,
                    opacity: 1.0,
                    position,
                }),
                ..TextSpec::new("")
            }),
            1.0,
        )
    };
    let measure = |raster: &TextRaster| {
        let y = mid_row(raster);
        let (mut ink, mut outline) = (0.0, 0.0);
        for x in 0..raster.image.width {
            let p = px(raster, x, y);
            ink += f64::from(p[3]) / 255.0;
            // White fill adds to red and green alike; the outline only to
            // red.
            outline += f64::from(p[0] - p[1]) / 255.0;
        }
        (ink, outline)
    };
    let plain = r.rasterize(&stem(TextSpec::new("")), 1.0);
    let (stem_ink, none) = measure(&plain);
    assert!((8.5..=9.5).contains(&stem_ink), "{stem_ink}");
    assert!(none.abs() < 1e-9);
    for (position, ink, outline) in [
        (StrokePosition::Outside, stem_ink + 8.0, 8.0),
        (StrokePosition::Center, stem_ink + 4.0, 8.0),
        (StrokePosition::Inside, stem_ink, 8.0),
    ] {
        let (got_ink, got_outline) = measure(&outlined(&mut r, position));
        assert!(
            (got_ink - ink).abs() < 0.6,
            "{position:?}: ink {got_ink}, want {ink}"
        );
        assert!(
            (got_outline - outline).abs() < 0.6,
            "{position:?}: outline {got_outline}, want {outline}"
        );
    }
}

#[test]
fn hollow_text_keeps_only_the_outline_ring() {
    let mut r = renderer();
    let raster = r.rasterize(
        &stem(TextSpec {
            fill: Fill::Solid([255, 255, 255, 0]),
            stroke: Some(Stroke {
                color: RED,
                width: 0.06,
                ..Stroke::default()
            }),
            ..TextSpec::new("")
        }),
        1.0,
    );
    let y = mid_row(&raster);
    let row: Vec<[u8; 4]> = (0..raster.image.width).map(|x| px(&raster, x, y)).collect();
    let inked: Vec<usize> = (0..row.len()).filter(|&i| row[i][3] > 0).collect();
    let (first, last) = (inked[0], *inked.last().unwrap());
    // The middle of the stem is empty; the ring on both sides is red.
    let middle = (first + last) / 2;
    assert_eq!(row[middle], [0, 0, 0, 0]);
    assert_eq!(row[first + 2], RED);
    assert_eq!(row[last - 2], RED);
}

/// Pixels of `a` and `b` (same spec apart from effects) compared in box
/// coordinates.
fn aligned<'a>(
    a: &'a TextRaster,
    b: &'a TextRaster,
) -> impl Iterator<Item = ([u8; 4], Option<[u8; 4]>)> + 'a {
    let (w, h) = (a.image.width as i32, a.image.height as i32);
    (0..h).flat_map(move |y| {
        (0..w).map(move |x| {
            let (bx, by) = (x - a.origin.0 + b.origin.0, y - a.origin.1 + b.origin.1);
            let other =
                (bx >= 0 && by >= 0 && bx < b.image.width as i32 && by < b.image.height as i32)
                    .then(|| px(b, bx as u32, by as u32));
            (px(a, x as u32, y as u32), other)
        })
    })
}

#[test]
fn inward_glow_lights_only_inside_the_letters() {
    let mut r = renderer();
    let base = TextSpec {
        text: "Oh".into(),
        size_px: 80.0,
        fill: Fill::Solid([0, 0, 0, 255]),
        ..TextSpec::new("")
    };
    let plain = r.rasterize(&base, 1.0);
    let glowing = r.rasterize(
        &TextSpec {
            glow: Some(Glow {
                color: RED,
                intensity: 1.0,
                radius: 0.15,
                direction: GlowDirection::Inward,
                ..Glow::default()
            }),
            ..base.clone()
        },
        1.0,
    );
    let mut lit = 0;
    for (g, p) in aligned(&glowing, &plain) {
        let p = p.unwrap_or([0; 4]);
        if g != p {
            // Light only where the letters are.
            assert!(p[3] > 0, "glow outside the glyphs: {g:?} over {p:?}");
            lit += 1;
        }
    }
    assert!(lit > 50, "{lit}");
}

#[test]
fn outward_glow_adds_light_around_the_letters() {
    let mut r = renderer();
    let raster = r.rasterize(
        &TextSpec {
            fill: Fill::Solid([0, 0, 0, 0]),
            glow: Some(Glow {
                color: [0, 255, 0, 255],
                intensity: 1.0,
                ..Glow::default()
            }),
            ..stem(TextSpec::new(""))
        },
        1.0,
    );
    // Pure light: colour with alpha 0 (the fill is transparent).
    assert!(raster.image.rgba.chunks(4).all(|p| p[3] == 0));
    assert!(raster.image.rgba.chunks(4).any(|p| p[1] > 200));
}

#[test]
fn a_shadow_at_45_degrees_falls_down_and_to_the_right() {
    let mut r = renderer();
    let raster = r.rasterize(
        &stem(TextSpec {
            shadow: Some(Shadow {
                color: [0, 0, 0, 255],
                opacity: 1.0,
                blur: 0.0,
                distance: 0.2,
                angle_deg: 45.0,
                include_box: false,
            }),
            ..TextSpec::new("")
        }),
        1.0,
    );
    // White fill: red channel; black shadow: alpha without colour.
    let (mut fill, mut shadow) = ((0.0, 0.0, 0.0), (0.0, 0.0, 0.0));
    for y in 0..raster.image.height {
        for x in 0..raster.image.width {
            let p = px(&raster, x, y);
            let (f, s) = (f64::from(p[0]), f64::from(p[3] - p[0]));
            fill = (
                fill.0 + f * f64::from(x),
                fill.1 + f * f64::from(y),
                fill.2 + f,
            );
            shadow = (
                shadow.0 + s * f64::from(x),
                shadow.1 + s * f64::from(y),
                shadow.2 + s,
            );
        }
    }
    let dx = shadow.0 / shadow.2 - fill.0 / fill.2;
    let dy = shadow.1 / shadow.2 - fill.1 / fill.2;
    let d = 20.0 / std::f64::consts::SQRT_2;
    assert!((dx - d).abs() < 1.5 && (dy - d).abs() < 1.5, "({dx}, {dy})");
}

#[test]
fn overlapping_per_line_boxes_do_not_darken() {
    let mut r = renderer();
    let raster = r.rasterize(
        &TextSpec {
            text: "Wide line\nline".into(),
            size_px: 40.0,
            line_spacing: 0.6,
            fill: Fill::Solid([255, 0, 0, 255]),
            background: Some(TextBox {
                color: [0, 0, 255, 255],
                opacity: 0.5,
                shape: BoxShape::PerLine,
                roundness: 0.0,
                ..TextBox::default()
            }),
            ..TextSpec::new("")
        },
        1.0,
    );
    let boxed: Vec<u8> = raster
        .image
        .rgba
        .chunks(4)
        .filter(|p| p[0] == 0 && p[3] > 0)
        .map(|p| p[3])
        .collect();
    assert!(!boxed.is_empty());
    // One opacity everywhere, overlap included.
    assert!(boxed.iter().all(|&a| a <= 128), "{:?}", boxed.iter().max());
    let solid = boxed.iter().filter(|&&a| a == 128).count();
    assert!(solid > boxed.len() * 9 / 10);
    // The padded boxes overlap (15 % of 40 px above and below each line).
    let layout = &raster.layout;
    assert!(layout.lines[1].top - layout.lines[0].bottom < 2.0 * 6.0);
}

#[test]
fn a_translucent_box_is_composited_once_border_included() {
    // Black at 50 % with a white border of 10 % (10 px): the border is
    // white at 50 %, not grey at 75 % (TX-013).
    let mut r = renderer();
    let raster = r.rasterize(
        &stem(TextSpec {
            background: Some(TextBox {
                color: [0, 0, 0, 255],
                opacity: 0.5,
                roundness: 0.0,
                border: Some(Border {
                    color: WHITE,
                    width: 0.1,
                }),
                ..TextBox::default()
            }),
            ..TextSpec::new("")
        }),
        1.0,
    );
    let y = mid_row(&raster);
    let left = (0..raster.image.width)
        .find(|&x| px(&raster, x, y)[3] > 0)
        .unwrap();
    let close = |a: [u8; 4], b: [u8; 4]| a.iter().zip(b).all(|(&a, b)| a.abs_diff(b) <= 1);
    let border = px(&raster, left + 5, y);
    assert!(close(border, [128, 128, 128, 128]), "border {border:?}");
    let fill = px(&raster, left + 15, y);
    assert!(close(fill, [0, 0, 0, 128]), "fill {fill:?}");
}

#[test]
fn a_large_outward_glow_fades_out_inside_the_raster() {
    // Radius 1 em at 400 px: the blur is capped at 256 px, the grown source
    // is not; the margin must cover both, or the glow ends in a hard edge.
    let mut r = renderer();
    let raster = r.rasterize(
        &TextSpec {
            text: "I".into(),
            size_px: 400.0,
            glow: Some(Glow {
                color: WHITE,
                intensity: 1.0,
                radius: 1.0,
                ..Glow::default()
            }),
            ..TextSpec::new("")
        },
        1.0,
    );
    assert_eq!(raster.scale, 1.0);
    let (w, h) = (raster.image.width, raster.image.height);
    let edge = (0..w)
        .flat_map(|x| [(x, 0), (x, h - 1)])
        .chain((0..h).flat_map(|y| [(0, y), (w - 1, y)]))
        .map(|(x, y)| px(&raster, x, y).into_iter().max().unwrap_or(0))
        .max()
        .unwrap_or(0);
    assert_eq!(edge, 0, "light on the raster's edge");
}

#[test]
fn decorations_follow_the_font_metrics() {
    let mut r = renderer();
    let base = TextSpec {
        text: "ab".into(),
        size_px: 60.0,
        ..TextSpec::new("")
    };
    let ink_rows = |raster: &TextRaster| -> Vec<i32> {
        (0..raster.image.height)
            .filter(|&y| (0..raster.image.width).any(|x| px(raster, x, y)[3] > 0))
            .map(|y| y as i32 - raster.origin.1)
            .collect()
    };
    let plain = r.rasterize(&base, 1.0);
    let underlined = r.rasterize(
        &TextSpec {
            decoration: Decoration {
                underline: true,
                strike: false,
            },
            ..base.clone()
        },
        1.0,
    );
    let baseline = plain.layout.lines[0].baseline.round() as i32;
    let below = |rows: &[i32]| rows.iter().filter(|&&y| y > baseline + 1).count();
    assert_eq!(below(&ink_rows(&plain)), 0);
    assert!(below(&ink_rows(&underlined)) >= 2);
    // A strike line crosses the gap between the letters at mid x-height.
    let struck = r.rasterize(
        &TextSpec {
            decoration: Decoration {
                underline: false,
                strike: true,
            },
            ..base
        },
        1.0,
    );
    let ink = |raster: &TextRaster| {
        raster
            .image
            .rgba
            .chunks(4)
            .map(|p| u64::from(p[3]))
            .sum::<u64>()
    };
    assert!(ink(&struck) > ink(&plain));
}

#[test]
fn letter_spacing_widens_lines_except_after_the_last_cluster() {
    let mut r = renderer();
    let base = TextSpec {
        text: "abcd".into(),
        size_px: 50.0,
        ..TextSpec::new("")
    };
    let plain = r.layout(&base);
    let spaced = r.layout(&TextSpec {
        letter_spacing: 0.1,
        ..base.clone()
    });
    let grown = spaced.size.0 - plain.size.0;
    // Three gaps of 5 px.
    assert!((grown - 15.0).abs() < 0.01, "{grown}");
    // Centred in a fixed box, the spaced line stays centred.
    let centred = r.layout(&TextSpec {
        letter_spacing: 0.1,
        align: TextAlign::Center,
        box_width_px: Some(400.0),
        ..base
    });
    let l = centred.lines[0];
    assert!((l.left + l.width / 2.0 - 200.0).abs() < 0.01);
}

#[test]
fn justified_lines_span_the_box_but_the_last_line() {
    let mut r = renderer();
    let layout = r.layout(&TextSpec {
        text: "one two three four five six seven eight".into(),
        size_px: 40.0,
        align: TextAlign::Justify,
        box_width_px: Some(330.0),
        ..TextSpec::new("")
    });
    assert!(layout.lines.len() >= 3, "{:?}", layout.lines);
    let (last, wrapped) = layout.lines.split_last().unwrap();
    for l in wrapped {
        assert_eq!((l.left, l.width), (0.0, 330.0));
    }
    assert!(last.width < 330.0);
    assert_eq!(last.left, 0.0);
    // The glyphs of a wrapped line really spread to the right edge.
    let line0_end = layout
        .glyphs
        .iter()
        .filter(|g| g.unit.line == 0)
        .map(|g| g.origin.0 + g.advance)
        .fold(0.0f32, f32::max);
    assert!(line0_end > 320.0, "{line0_end}");
}

#[test]
fn glyphs_report_their_position_and_animation_units() {
    let mut r = renderer();
    let text = "Hi there\nyou";
    let layout = r.layout(&TextSpec {
        text: text.into(),
        size_px: 40.0,
        ..TextSpec::new("")
    });
    // One glyph per character here (Latin, no ligatures); the line break
    // has none.
    assert_eq!(layout.glyphs.len(), text.chars().count() - 1);
    let you = text.find("you").unwrap() as u32;
    for g in &layout.glyphs {
        let line = u32::from(g.cluster >= you);
        assert_eq!(g.unit.line, line, "{g:?}");
        assert!(g.size == 40.0);
    }
    let unit = |byte: usize| {
        layout
            .glyphs
            .iter()
            .find(|g| g.cluster == byte as u32)
            .unwrap()
            .unit
    };
    assert_eq!(unit(0).word, 0);
    assert_eq!(unit(2).word, 0); // the space belongs to "Hi"
    assert_eq!(unit(3).word, 1);
    assert_eq!(unit(you as usize).word, 2);
    assert_eq!(unit(3).grapheme, 3);
    // Pen positions advance along each line, on its baseline.
    let line0: Vec<_> = layout.glyphs.iter().filter(|g| g.unit.line == 0).collect();
    assert!(line0.windows(2).all(|w| w[1].origin.0 > w[0].origin.0));
    assert!(line0.iter().all(|g| g.origin.1 == layout.lines[0].baseline));
}

/// Ink bounding box in box coordinates (scale 1).
fn ink_box(r: &TextRaster) -> (i32, i32, i32, i32) {
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for y in 0..r.image.height {
        for x in 0..r.image.width {
            if px(r, x, y)[3] > 0 {
                let (bx, by) = (x as i32 - r.origin.0, y as i32 - r.origin.1);
                x0 = x0.min(bx);
                y0 = y0.min(by);
                x1 = x1.max(bx);
                y1 = y1.max(by);
            }
        }
    }
    (x0, y0, x1, y1)
}

#[test]
fn curved_text_bends_into_an_arch_or_a_bowl() {
    let mut r = renderer();
    let base = TextSpec {
        text: "IIIIIIIIIIII".into(),
        size_px: 50.0,
        ..TextSpec::new("")
    };
    let straight = ink_box(&r.rasterize(&base, 1.0));
    let arch_raster = r.rasterize(
        &TextSpec {
            curve: 0.4,
            ..base.clone()
        },
        1.0,
    );
    let arch = ink_box(&arch_raster);
    let bowl = ink_box(&r.rasterize(
        &TextSpec {
            curve: -0.4,
            ..base.clone()
        },
        1.0,
    ));
    let height = |b: (i32, i32, i32, i32)| b.3 - b.1;
    assert!(
        height(arch) > height(straight) + 20,
        "{arch:?} {straight:?}"
    );
    // The arch's ends drop below the straight baseline; the bowl's rise.
    assert!(arch.3 > straight.3 + 10);
    assert!(bowl.1 < straight.1 - 10);
    // On the right of an arch a glyph is turned clockwise: a lone stem at
    // the right end leans right at the top.
    let ends = r.rasterize(
        &TextSpec {
            text: "I          I".into(),
            curve: 0.25,
            ..base.clone()
        },
        1.0,
    );
    let img = &ends.image;
    let centre = ends.origin.0 + (ends.layout.size.0 / 2.0) as i32;
    let right: Vec<(u32, u32)> = (0..img.height)
        .flat_map(|y| (0..img.width).map(move |x| (x, y)))
        .filter(|&(x, y)| x as i32 > centre && px(&ends, x, y)[3] > 128)
        .collect();
    let top = right.iter().min_by_key(|p| p.1).unwrap();
    let bottom = right.iter().max_by_key(|p| p.1).unwrap();
    assert!(top.0 > bottom.0 + 8, "top {top:?} bottom {bottom:?}");
    // A full circle closes: ink all round.
    let circle = ink_box(&r.rasterize(&TextSpec { curve: 1.0, ..base }, 1.0));
    let (w, h) = (circle.2 - circle.0, circle.3 - circle.1);
    assert!((w - h).abs() < 12, "{circle:?}");
}

#[test]
fn a_linear_gradient_runs_left_to_right_at_zero_degrees() {
    let mut r = renderer();
    let raster = r.rasterize(
        &TextSpec {
            text: "HHHHHH".into(),
            size_px: 50.0,
            fill: Fill::Linear(LinearGradient {
                angle_deg: 0.0,
                stops: vec![
                    GradientStop {
                        at: 0.0,
                        color: RED,
                    },
                    GradientStop {
                        at: 1.0,
                        color: [0, 0, 255, 255],
                    },
                ],
                extent: FillExtent::Block,
            }),
            ..TextSpec::new("")
        },
        1.0,
    );
    let (x0, _, x1, _) = ink_box(&raster);
    let column_hue = |bx: i32| -> (u32, u32) {
        let x = (bx + raster.origin.0) as u32;
        (0..raster.image.height)
            .map(|y| px(&raster, x, y))
            .filter(|p| p[3] == 255)
            .fold((0, 0), |(r, b), p| {
                (r + u32::from(p[0]), b + u32::from(p[2]))
            })
    };
    let left = column_hue(x0 + 3);
    let right = column_hue(x1 - 3);
    assert!(left.0 > left.1 * 4, "{left:?}");
    assert!(right.1 > right.0 * 4, "{right:?}");
}

#[test]
fn every_style_field_is_part_of_the_cache_key() {
    let mut r = renderer();
    let base = TextSpec {
        stroke: Some(Stroke::default()),
        ..TextSpec::new("key")
    };
    let a = r.rasterize(&base, 1.0);
    assert!(Arc::ptr_eq(&a, &r.rasterize(&base.clone(), 1.0)));
    let variants = [
        TextSpec {
            stroke: Some(Stroke {
                width: 0.07,
                ..Stroke::default()
            }),
            ..base.clone()
        },
        TextSpec {
            stroke: Some(Stroke {
                position: StrokePosition::Inside,
                ..Stroke::default()
            }),
            ..base.clone()
        },
        TextSpec {
            letter_spacing: 0.01,
            ..base.clone()
        },
        TextSpec {
            curve: 0.1,
            ..base.clone()
        },
        TextSpec {
            glow: Some(Glow::default()),
            ..base.clone()
        },
        TextSpec {
            shadow: Some(Shadow::default()),
            ..base.clone()
        },
        TextSpec {
            background: Some(TextBox::default()),
            ..base.clone()
        },
        TextSpec {
            bubble: Some(TextBubble::default()),
            ..base.clone()
        },
        TextSpec {
            decoration: Decoration {
                underline: true,
                strike: false,
            },
            ..base.clone()
        },
    ];
    for v in &variants {
        let b = r.rasterize(v, 1.0);
        assert!(!Arc::ptr_eq(&a, &b), "{v:?}");
        assert_ne!(a.image.rgba, b.image.rgba, "{v:?}");
    }
}

#[test]
fn a_bubble_with_a_tail_reaches_its_tip() {
    let mut r = renderer();
    let spec = TextSpec {
        text: "Hello".into(),
        size_px: 40.0,
        fill: Fill::Solid([0, 0, 0, 255]),
        bubble: Some(TextBubble {
            spec: freecut_render::shape::bubble::BubbleSpec {
                kind: BubbleKind::Rounded,
                tail: Some(Tail {
                    tip: Point::new(-30.0, 120.0),
                    base: 0.24,
                    bend: 0.2,
                }),
                ..Default::default()
            },
            ..TextBubble::default()
        }),
        ..TextSpec::new("")
    };
    let raster = r.rasterize(&spec, 1.0);
    let layout = &raster.layout;
    let tip = (layout.size.0 / 2.0 - 30.0, layout.size.1 / 2.0 + 120.0);
    let (_, _, _, bottom) = ink_box(&raster);
    assert!((bottom as f32 - tip.1).abs() <= 2.0, "{bottom} vs {tip:?}");
    // The bubble is opaque white around the black text.
    let corner = px(
        &raster,
        (raster.origin.0 - 4) as u32,
        (raster.origin.1 + 5) as u32,
    );
    assert_eq!(corner, WHITE);
}

#[test]
fn rasters_under_budget_keep_the_requested_scale() {
    // Curves, long tails and large glows stay at output resolution (TX-004)
    // when the image they make is well within the budget.
    let mut r = renderer();
    let curved = TextSpec {
        text: "CURVED TITLE TEXT".into(),
        size_px: 120.0,
        curve: 0.2,
        ..TextSpec::new("")
    };
    let tailed = TextSpec {
        text: "Hello".into(),
        bubble: Some(TextBubble {
            spec: freecut_render::shape::bubble::BubbleSpec {
                kind: BubbleKind::Rounded,
                tail: Some(Tail {
                    tip: Point::new(0.0, 40.0 * 64.0),
                    base: 0.24,
                    bend: 0.2,
                }),
                ..Default::default()
            },
            ..TextBubble::default()
        }),
        ..TextSpec::new("")
    };
    let glowing = TextSpec {
        text: "I".into(),
        size_px: 600.0,
        glow: Some(Glow {
            radius: 1.0,
            ..Glow::default()
        }),
        ..TextSpec::new("")
    };
    let cases = [
        ("curved", curved, &[1.0, 2.0][..]),
        ("tailed", tailed, &[1.0, 2.0]),
        // At scale 2 this one is over budget (about 14 Mpx).
        ("glow", glowing, &[1.0]),
    ];
    for (name, spec, scales) in cases {
        for &scale in scales {
            let raster = r.rasterize(&spec, scale);
            let (w, h) = (raster.image.width, raster.image.height);
            assert!(w * h < 1 << 23, "{name} at {scale}: {w}x{h}");
            assert_eq!(raster.scale, scale, "{name}: {w}x{h}");
        }
    }
}

#[test]
fn rasters_over_budget_are_made_smaller_but_not_cropped() {
    let mut r = renderer();
    let spec = TextSpec {
        text: "WWWWWW".into(),
        size_px: 1000.0,
        glow: Some(Glow::default()),
        ..TextSpec::new("")
    };
    let raster = r.rasterize(&spec, 4.0);
    let (w, h) = (raster.image.width, raster.image.height);
    assert!(raster.scale < 4.0);
    assert!(w <= 8192 && h <= 8192, "{w}x{h}");
    assert!(
        f64::from(w) * f64::from(h) <= 1.05 * f64::from(1u32 << 23),
        "{w}x{h}"
    );
    // The glyphs are all inside: the outermost columns carry no ink.
    let column = |x: u32| (0..h).all(|y| px(&raster, x, y)[3] == 0);
    assert!(column(0) && column(w - 1));
}

fn styled_title() -> TextSpec {
    TextSpec {
        text: "Styled title".into(),
        size_px: 120.0,
        bold: true,
        stroke: Some(Stroke::default()),
        shadow: Some(Shadow::default()),
        glow: Some(Glow {
            color: [255, 220, 120, 255],
            ..Glow::default()
        }),
        ..TextSpec::new("")
    }
}

fn text_plan(spec: TextSpec, background: [u8; 4]) -> (freecut_timeline::plan::FramePlan, Images) {
    let mut p = plan(
        640,
        360,
        vec![common::layer(
            LayerSource::Text {
                data: Arc::new(TextClipData::default()),
                scale_px: 1.0,
            },
            (640, 360),
            DMat3::IDENTITY,
        )],
    );
    p.canvas.background = background;
    let mut images = Images::default();
    images.texts.insert(
        clip(1),
        TextLayer {
            spec,
            layer_to_canvas: DMat3::from_translation(DVec2::new(320.0, 180.0))
                * DMat3::from_angle(0.1),
            anchor: TextAnchor::Center,
            motion: None,
        },
    );
    (p, images)
}

#[test]
fn glow_adds_light_over_black_and_saturates_over_white() {
    let glow_only = TextSpec {
        text: "glow".into(),
        size_px: 80.0,
        fill: Fill::Solid([0, 0, 0, 0]),
        glow: Some(Glow {
            intensity: 1.0,
            ..Glow::default()
        }),
        ..TextSpec::new("")
    };
    let (p, mut images) = text_plan(glow_only.clone(), [0, 0, 0, 255]);
    let mut comp = Compositor::new(FontRegistry::bundled());
    let black = comp.render(&p, 640, 360, &mut images);
    assert!(black.rgba.chunks(4).any(|px| px[0] > 100));
    assert!(black.rgba.chunks(4).all(|px| px[3] == 255));
    let (p, mut images) = text_plan(glow_only, [255, 255, 255, 255]);
    let white = comp.render(&p, 640, 360, &mut images);
    assert!(white.rgba.chunks(4).all(|px| px == [255, 255, 255, 255]));
}

#[test]
fn transparent_parts_of_a_text_layer_never_darken() {
    // A mid-grey background: only the text's own pixels may change it.
    let (p, mut images) = text_plan(
        TextSpec {
            text: "Hi".into(),
            size_px: 60.0,
            ..TextSpec::new("")
        },
        [128, 128, 128, 255],
    );
    let frame = Compositor::new(FontRegistry::bundled()).render(&p, 640, 360, &mut images);
    // White text over grey only brightens.
    assert!(frame.rgba.chunks(4).all(|px| px[0] >= 128 && px[3] == 255));
    assert!(frame.rgba.chunks(4).any(|px| px[0] == 255));
}

#[test]
fn styled_text_renders_byte_identically_on_two_threads() {
    let spec = TextSpec {
        bubble: Some(TextBubble::default()),
        ..styled_title()
    };
    let run = move |threads: usize| {
        let spec = spec.clone();
        std::thread::spawn(move || {
            let (p, mut images) = text_plan(spec, [20, 30, 40, 255]);
            Compositor::new(FontRegistry::bundled())
                .with_threads(threads)
                .render(&p, 640, 360, &mut images)
        })
    };
    let (a, b, c) = (run(1), run(1), run(4));
    let (a, b, c) = (a.join().unwrap(), b.join().unwrap(), c.join().unwrap());
    assert!(a == b, "two threads differ");
    assert!(a == c, "band threads change the output");
}

/// Run with `cargo test -p freecut-render --release -- --ignored`.
#[test]
#[ignore = "timing; meaningful only in release builds"]
fn a_1080p_styled_title_renders_fast() {
    use std::time::Instant;
    let mut p = plan(
        1920,
        1080,
        vec![common::layer(
            LayerSource::Text {
                data: Arc::new(TextClipData::default()),
                scale_px: 1.0,
            },
            (1920, 1080),
            DMat3::IDENTITY,
        )],
    );
    p.canvas.background = [10, 10, 10, 255];
    let mut images = Images::default();
    images.texts.insert(
        clip(1),
        TextLayer {
            spec: TextSpec {
                bubble: None,
                ..styled_title()
            },
            layer_to_canvas: DMat3::from_translation(DVec2::new(960.0, 540.0)),
            anchor: TextAnchor::Center,
            motion: None,
        },
    );
    for threads in [1, 8] {
        let mut comp = Compositor::new(FontRegistry::bundled()).with_threads(threads);
        let start = Instant::now();
        comp.render(&p, 1920, 1080, &mut images);
        let first = start.elapsed();
        let runs = 10;
        let start = Instant::now();
        for _ in 0..runs {
            comp.render(&p, 1920, 1080, &mut images);
        }
        let cached = start.elapsed() / runs;
        println!(
            "1080p styled title, {threads} thread(s): first frame {first:?}, cached {cached:?}"
        );
        assert!(cached.as_millis() < 30, "{cached:?}");
        assert!(first.as_millis() < 400, "{first:?}");
    }
}
