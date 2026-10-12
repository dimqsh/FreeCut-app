//! Golden rasters of styled text: one small sample per style layer and per
//! bubble kind, rendered with the bundled fonts only and compared with the
//! checked-in PNGs in `tests/golden/` (premultiplied RGBA, ±1 per
//! channel).
//!
//! To regenerate after an intended change, run with
//! `FREECUT_BLESS_GOLDENS=1` and review the new PNGs before committing.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

use std::path::PathBuf;

use freecut_render::shape::Point;
use freecut_render::shape::bubble::{BubbleKind, BubbleSpec, Side, Tail};
use freecut_render::text::{
    Border, BoxHeight, BoxShape, Decoration, Fill, FillExtent, FontRegistry, Glow, GlowDirection,
    GradientStop, LinearGradient, RadialGradient, Shadow, Stroke, StrokePosition, TextBox,
    TextBubble, TextRenderer, TextSpec,
};

const INK: [u8; 4] = [250, 250, 250, 255];
const DARK: [u8; 4] = [20, 24, 32, 255];
const ACCENT: [u8; 4] = [255, 120, 40, 255];

fn base(text: &str) -> TextSpec {
    TextSpec {
        size_px: 32.0,
        bold: true,
        fill: Fill::Solid(INK),
        ..TextSpec::new(text)
    }
}

fn stroke(position: StrokePosition) -> TextSpec {
    TextSpec {
        stroke: Some(Stroke {
            color: ACCENT,
            width: 0.08,
            opacity: 1.0,
            position,
        }),
        ..base("Ag")
    }
}

fn bubble(kind: BubbleKind) -> TextSpec {
    let tail = Tail {
        tip: Point::new(-24.0, 64.0),
        base: 0.24,
        bend: 0.2,
    };
    TextSpec {
        fill: Fill::Solid(DARK),
        bubble: Some(TextBubble {
            spec: BubbleSpec {
                kind,
                tail: Some(tail),
                seed: 7,
                ..BubbleSpec::default()
            },
            ..TextBubble::default()
        }),
        ..base("Hey")
    }
}

fn samples() -> Vec<(&'static str, TextSpec)> {
    let stops = vec![
        GradientStop {
            at: 0.0,
            color: ACCENT,
        },
        GradientStop {
            at: 1.0,
            color: [60, 140, 255, 255],
        },
    ];
    vec![
        ("fill_solid", base("Fill")),
        (
            "fill_linear",
            TextSpec {
                fill: Fill::Linear(LinearGradient {
                    angle_deg: 30.0,
                    stops: stops.clone(),
                    extent: FillExtent::Block,
                }),
                ..base("Grad")
            },
        ),
        (
            "fill_radial_lines",
            TextSpec {
                fill: Fill::Radial(RadialGradient {
                    center: (0.5, 0.5),
                    spread: 1.0,
                    stops,
                    extent: FillExtent::Line,
                }),
                ..base("Ra\ndial")
            },
        ),
        ("outline_outside", stroke(StrokePosition::Outside)),
        ("outline_center", stroke(StrokePosition::Center)),
        ("outline_inside", stroke(StrokePosition::Inside)),
        (
            "shadow",
            TextSpec {
                shadow: Some(Shadow {
                    opacity: 0.8,
                    ..Shadow::default()
                }),
                ..base("Shade")
            },
        ),
        (
            "glow_outward",
            TextSpec {
                glow: Some(Glow {
                    color: ACCENT,
                    ..Glow::default()
                }),
                ..base("Glow")
            },
        ),
        (
            "glow_inward",
            TextSpec {
                fill: Fill::Solid(DARK),
                glow: Some(Glow {
                    color: ACCENT,
                    intensity: 1.0,
                    radius: 0.2,
                    direction: GlowDirection::Inward,
                    ..Glow::default()
                }),
                ..base("Glow")
            },
        ),
        (
            "box_block",
            TextSpec {
                background: Some(TextBox::default()),
                ..base("Box")
            },
        ),
        (
            "box_per_line_border",
            TextSpec {
                line_spacing: 0.8,
                background: Some(TextBox {
                    shape: BoxShape::PerLine,
                    color: [40, 90, 220, 255],
                    border: Some(Border {
                        color: INK,
                        width: 0.05,
                    }),
                    ..TextBox::default()
                }),
                ..base("Per line\nboxes")
            },
        ),
        (
            "box_per_char_lower_half",
            TextSpec {
                fill: Fill::Solid(DARK),
                background: Some(TextBox {
                    shape: BoxShape::PerChar,
                    height: BoxHeight::LowerHalf,
                    color: [255, 230, 0, 255],
                    opacity: 1.0,
                    pad_x: 0.05,
                    ..TextBox::default()
                }),
                ..base("Tiles")
            },
        ),
        (
            "decorations",
            TextSpec {
                decoration: Decoration {
                    underline: true,
                    strike: true,
                },
                letter_spacing: 0.1,
                ..base("Deco")
            },
        ),
        (
            "curve_arch",
            TextSpec {
                curve: 0.35,
                decoration: Decoration {
                    underline: true,
                    strike: false,
                },
                ..base("Curved text")
            },
        ),
        (
            "curve_bowl",
            TextSpec {
                curve: -0.35,
                ..base("Curved text")
            },
        ),
        ("bubble_rounded", bubble(BubbleKind::Rounded)),
        ("bubble_oval", bubble(BubbleKind::Oval)),
        ("bubble_thought", bubble(BubbleKind::Thought)),
        ("bubble_shout", bubble(BubbleKind::Shout)),
        ("bubble_square", bubble(BubbleKind::Square)),
        ("bubble_tag", bubble(BubbleKind::Tag)),
        (
            "bubble_arrow_tag",
            bubble(BubbleKind::ArrowTag { point: Side::Left }),
        ),
        ("bubble_ticket", bubble(BubbleKind::Ticket)),
    ]
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

#[test]
fn styled_text_matches_the_goldens() {
    let bless = std::env::var_os("FREECUT_BLESS_GOLDENS").is_some();
    let mut renderer = TextRenderer::new(FontRegistry::bundled());
    let mut failures = Vec::new();
    for (name, spec) in samples() {
        let raster = renderer.rasterize(&spec, 1.0);
        let img = &raster.image;
        assert!(!img.is_empty(), "{name}: nothing drawn");
        let path = golden_dir().join(format!("{name}.png"));
        if bless {
            std::fs::create_dir_all(golden_dir()).unwrap();
            image::save_buffer(
                &path,
                &img.rgba,
                img.width,
                img.height,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
            continue;
        }
        let Ok(golden) = image::open(&path) else {
            failures.push(format!("{name}: missing {}", path.display()));
            continue;
        };
        let golden = golden.to_rgba8();
        if golden.dimensions() != (img.width, img.height) {
            failures.push(format!(
                "{name}: size {:?}, golden {:?}",
                (img.width, img.height),
                golden.dimensions()
            ));
            continue;
        }
        let worst = golden
            .as_raw()
            .iter()
            .zip(&img.rgba)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        if worst > 1 {
            failures.push(format!("{name}: differs by up to {worst}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
