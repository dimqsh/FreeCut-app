//! The timeline's text clips as [`TextSpec`]s (TX-002 … TX-016, TX-055).
//!
//! The model stores fixed-point values: lengths in ref px (a pixel of a
//! canvas whose short edge is 1080 px), ratios in per-mille, angles in
//! tenths of a degree, and optional layers as `Layer { on, value }`. The
//! renderer works in layout px (canvas pixels at 100 % clip scale) and
//! `f32`, with fractions of the font size for lengths the spec defines
//! relative to it, and `Option` for layers. This is the one place that
//! converts between the two (the render edge):
//!
//! - ref px × `scale_px` → layout px (font size, fixed width, tail tip);
//! - per-mille → fraction (`PerMille::as_f64`), so a 60 ‰ outline width is
//!   `0.06` of the font size and a 500 ‰ opacity is `0.5`;
//! - tenths of a degree → degrees;
//! - a layer that is off → `None` (its stored value is kept by the model and
//!   simply not drawn); the curve maps to `0.0` while off;
//! - the case transform is applied to the content here, before layout
//!   (TX-008), with no language tag until TX-006 adds one to the style;
//! - `justify` wins over the alignment (TX-005).
//!
//! The vertical anchor ([`TextLayout::v_anchor`]) is a placement concern,
//! not part of the spec: [`crate::cpu::TextLayer::from_plan`] maps it onto
//! [`crate::cpu::TextAnchor`].
//!
//! [`TextLayout::v_anchor`]: freecut_timeline::model::TextLayout::v_anchor

use freecut_timeline::model::{self, HAlign, Rgba8, TextClipData, case};

use super::style::{GradientStop, LinearGradient, RadialGradient};
use super::{
    Border, BoxHeight, BoxShape, Decoration, Fill, FillExtent, Glow, GlowDirection, Shadow, Stroke,
    StrokePosition, TextAlign, TextBox, TextBubble, TextSpec,
};
use crate::shape::Point;
use crate::shape::bubble::{BubbleKind, BubbleSpec, Side, Tail};

fn rgba(c: Rgba8) -> [u8; 4] {
    [c.r, c.g, c.b, c.a]
}

/// A per-mille ratio as a fraction (1000 → 1.0).
fn frac(v: model::PerMille) -> f32 {
    v.as_f64() as f32
}

fn signed(v: model::SignedPerMille) -> f32 {
    v.as_f64() as f32
}

fn degrees(a: model::DeciDegrees) -> f32 {
    f32::from(a.tenths()) / 10.0
}

fn stops(stops: &model::GradientStops) -> Vec<GradientStop> {
    stops
        .as_slice()
        .iter()
        .map(|s| GradientStop {
            at: frac(s.at),
            color: rgba(s.color),
        })
        .collect()
}

fn extent(e: model::FillExtent) -> FillExtent {
    match e {
        model::FillExtent::Block => FillExtent::Block,
        model::FillExtent::Line => FillExtent::Line,
    }
}

/// The model's fill (TX-004, TX-014) as the renderer's.
fn fill(f: &model::Fill) -> Fill {
    match f {
        model::Fill::Solid(c) => Fill::Solid(rgba(*c)),
        model::Fill::LinearGradient(g) => Fill::Linear(LinearGradient {
            angle_deg: degrees(g.angle),
            stops: stops(&g.stops),
            extent: extent(g.extent),
        }),
        model::Fill::RadialGradient(g) => Fill::Radial(RadialGradient {
            center: (frac(g.center.x), frac(g.center.y)),
            spread: frac(g.spread),
            stops: stops(&g.stops),
            extent: extent(g.extent),
        }),
    }
}

/// The outline (TX-010).
fn stroke(s: &model::Stroke) -> Stroke {
    Stroke {
        color: rgba(s.color),
        width: frac(s.width),
        opacity: frac(s.opacity),
        position: match s.position {
            model::StrokePosition::Outside => StrokePosition::Outside,
            model::StrokePosition::Center => StrokePosition::Center,
            model::StrokePosition::Inside => StrokePosition::Inside,
        },
    }
}

/// The drop shadow (TX-011).
fn shadow(s: &model::Shadow) -> Shadow {
    Shadow {
        color: rgba(s.color),
        opacity: frac(s.opacity),
        blur: frac(s.blur),
        distance: frac(s.distance),
        angle_deg: degrees(s.angle),
        include_box: s.include_box,
    }
}

/// The glow (TX-012).
fn glow(g: &model::Glow) -> Glow {
    Glow {
        color: rgba(g.color),
        intensity: frac(g.intensity),
        radius: frac(g.radius),
        offset: frac(g.offset),
        angle_deg: degrees(g.angle),
        direction: match g.direction {
            model::GlowDirection::Outward => GlowDirection::Outward,
            model::GlowDirection::Inward => GlowDirection::Inward,
            model::GlowDirection::Both => GlowDirection::Both,
        },
    }
}

/// A box or bubble border, `None` while its switch is off.
fn border(b: &model::Layer<model::Border>) -> Option<Border> {
    b.active().map(|b| Border {
        color: rgba(b.color),
        width: frac(b.width),
    })
}

/// The background box (TX-013).
fn text_box(b: &model::TextBox) -> TextBox {
    TextBox {
        color: rgba(b.color),
        opacity: frac(b.opacity),
        shape: match b.shape {
            model::BoxShape::Block => BoxShape::Block,
            model::BoxShape::PerLine => BoxShape::PerLine,
            model::BoxShape::PerChar => BoxShape::PerChar,
        },
        pad_x: frac(b.pad_x),
        pad_y: frac(b.pad_y),
        roundness: frac(b.roundness),
        height: match b.height {
            model::BoxHeight::Full => BoxHeight::Full,
            model::BoxHeight::LowerHalf => BoxHeight::LowerHalf,
        },
        border: border(&b.border),
    }
}

fn bubble_kind(k: model::BubbleKind) -> BubbleKind {
    match k {
        model::BubbleKind::Rounded => BubbleKind::Rounded,
        model::BubbleKind::Oval => BubbleKind::Oval,
        model::BubbleKind::Thought => BubbleKind::Thought,
        model::BubbleKind::Shout => BubbleKind::Shout,
        model::BubbleKind::Square => BubbleKind::Square,
        model::BubbleKind::Tag => BubbleKind::Tag,
        model::BubbleKind::ArrowTag { point } => BubbleKind::ArrowTag {
            point: match point {
                model::Side::Left => Side::Left,
                model::Side::Right => Side::Right,
            },
        },
        model::BubbleKind::Ticket => BubbleKind::Ticket,
    }
}

/// The bubble (TX-055); `px` turns ref px into layout px (the tail tip).
fn text_bubble(b: &model::TextBubble, px: impl Fn(f64) -> f32) -> TextBubble {
    let spec = &b.spec;
    TextBubble {
        spec: BubbleSpec {
            kind: bubble_kind(spec.kind),
            tail: spec.tail.active().map(|t| Tail {
                tip: Point::new(px(t.tip.x.as_f64()), px(t.tip.y.as_f64())),
                base: frac(t.base),
                bend: signed(t.bend),
            }),
            spikes: spec.spikes,
            spike_depth: frac(spec.spike_depth),
            jitter: frac(spec.jitter),
            seed: spec.seed,
            lobe: frac(spec.lobe),
            trail: spec.trail,
        },
        fill: rgba(b.fill),
        opacity: frac(b.opacity),
        pad_x: frac(b.pad_x),
        pad_y: frac(b.pad_y),
        roundness: frac(b.roundness),
        border: border(&b.border),
    }
}

impl TextSpec {
    /// The spec that draws `data` on a canvas with `scale_px` canvas pixels
    /// per ref px ([`LayerSource::Text::scale_px`]). Every length in ref px
    /// (font size, fixed box width, bubble tail tip) is multiplied by it, so
    /// the text is laid out in canvas pixels at 100 % clip scale; the clip's
    /// own scale is applied later by the layer transform and never changes
    /// line breaks (TX-004). Lengths relative to the font size stay
    /// fractions of it (see the module docs).
    ///
    /// CRLF line breaks become LF (TX-002 stores text as typed), then the
    /// case transform is applied (TX-008).
    ///
    /// [`LayerSource::Text::scale_px`]: freecut_timeline::plan::LayerSource::Text
    pub fn from_clip(data: &TextClipData, scale_px: f64) -> TextSpec {
        let px = |ref_px: f64| (ref_px * scale_px) as f32;
        let style = &data.style;
        let layout = &data.layout;
        let typed = data.content.as_str();
        let text = if typed.contains("\r\n") {
            std::borrow::Cow::Owned(typed.replace("\r\n", "\n"))
        } else {
            std::borrow::Cow::Borrowed(typed)
        };
        TextSpec {
            text: case::display(&text, style.case, None).into_owned(),
            family: style.font.family.clone(),
            size_px: px(style.size.as_f64()),
            fill: fill(&style.fill),
            bold: style.bold,
            italic: style.italic,
            align: if layout.justify {
                TextAlign::Justify
            } else {
                match layout.align {
                    HAlign::Left => TextAlign::Left,
                    HAlign::Center => TextAlign::Center,
                    HAlign::Right => TextAlign::Right,
                }
            },
            box_width_px: layout.box_width.map(|w| px(w.as_f64())),
            line_spacing: frac(layout.line_spacing),
            letter_spacing: signed(style.letter_spacing),
            decoration: Decoration {
                underline: style.decoration.underline,
                strike: style.decoration.strike,
            },
            stroke: style.stroke.active().map(stroke),
            shadow: style.shadow.active().map(shadow),
            glow: style.glow.active().map(glow),
            background: style.background.active().map(text_box),
            bubble: style.bubble.active().map(|b| text_bubble(b, px)),
            curve: style.curve.active().map_or(0.0, |c| signed(*c)),
            ligatures: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use freecut_timeline::model::{
        BubbleSpec as MBubbleSpec, CaseTransform, DeciDegrees, FillCenter, FontRef, GradientStops,
        LangTag, Layer, PerMille, RefPx, RichText, SignedPerMille, TailTip, TextLayout, TextStyle,
        VAnchor,
    };

    use super::*;

    #[test]
    fn model_units_become_canvas_pixels() {
        let data = TextClipData {
            content: RichText::new("one\r\ntwo"),
            style: TextStyle {
                font: FontRef::new("Some Family"),
                size: RefPx::from_hundredths(6450),
                fill: model::Fill::Solid(Rgba8::new(10, 20, 30, 40)),
                bold: false,
                italic: true,
                ..TextStyle::default()
            },
            layout: TextLayout {
                align: HAlign::Right,
                box_width: Some(RefPx::from_px(300)),
                line_spacing: PerMille::new(1500),
                ..TextLayout::default()
            },
            ..TextClipData::default()
        };
        let spec = TextSpec::from_clip(&data, 2.0);
        assert_eq!(
            spec,
            TextSpec {
                text: "one\ntwo".into(),
                family: "Some Family".into(),
                size_px: 129.0,
                fill: Fill::Solid([10, 20, 30, 40]),
                bold: false,
                italic: true,
                align: TextAlign::Right,
                box_width_px: Some(600.0),
                line_spacing: 1.5,
                ..TextSpec::new("")
            }
        );
    }

    #[test]
    fn defaults_map_to_centred_auto_width_bold_white_without_layers() {
        let spec = TextSpec::from_clip(&TextClipData::default(), 0.5);
        assert_eq!(spec.text, TextClipData::PLACEHOLDER);
        assert_eq!(spec.family, FontRef::DEFAULT_FAMILY);
        assert_eq!(spec.size_px, 32.0);
        assert_eq!(spec.fill, Fill::Solid([255; 4]));
        assert!(spec.bold && !spec.italic);
        assert_eq!(spec.align, TextAlign::Center);
        assert_eq!(spec.box_width_px, None);
        assert_eq!(spec.line_spacing, 1.0);
        assert_eq!(spec.letter_spacing, 0.0);
        assert_eq!(spec.decoration, Decoration::default());
        assert_eq!(
            (spec.stroke, spec.shadow, spec.glow),
            (None, None, None),
            "every optional layer of the factory style is off"
        );
        assert_eq!((spec.background, spec.bubble), (None, None));
        assert_eq!(spec.curve, 0.0);
    }

    #[test]
    fn gradient_fills_keep_their_stops_angle_and_centre() {
        let stops = GradientStops::two(Rgba8::new(255, 0, 0, 255), Rgba8::new(0, 0, 255, 128));
        let linear = fill(&model::Fill::LinearGradient(model::LinearGradient {
            angle: DeciDegrees::new(455),
            stops: stops.clone(),
            extent: model::FillExtent::Line,
        }));
        let Fill::Linear(g) = linear else {
            panic!("{linear:?}");
        };
        assert_eq!(g.angle_deg, 45.5);
        assert_eq!(g.extent, FillExtent::Line);
        assert_eq!(g.stops.len(), 2);
        assert_eq!((g.stops[0].at, g.stops[1].at), (0.0, 1.0));
        assert_eq!(g.stops[1].color, [0, 0, 255, 128]);
        let radial = fill(&model::Fill::RadialGradient(model::RadialGradient {
            center: FillCenter::MIDDLE,
            spread: PerMille::new(1500),
            stops,
            extent: model::FillExtent::Block,
        }));
        let Fill::Radial(g) = radial else {
            panic!("{radial:?}");
        };
        assert_eq!((g.center, g.spread), ((0.5, 0.5), 1.5));
        assert_eq!(g.extent, FillExtent::Block);
    }

    /// A style with every layer on and every value away from its default.
    fn styled() -> TextStyle {
        let c = |r, g, b| Rgba8::new(r, g, b, 255);
        TextStyle {
            decoration: model::Decoration {
                underline: true,
                strike: true,
            },
            case: CaseTransform::Upper,
            letter_spacing: SignedPerMille::new(-150),
            stroke: Layer::on(model::Stroke {
                color: c(1, 2, 3),
                width: PerMille::new(120),
                opacity: PerMille::new(750),
                position: model::StrokePosition::Center,
            }),
            shadow: Layer::on(model::Shadow {
                color: c(4, 5, 6),
                opacity: PerMille::new(250),
                blur: PerMille::new(300),
                distance: PerMille::new(400),
                angle: DeciDegrees::new(1355),
                include_box: true,
            }),
            glow: Layer::on(model::Glow {
                color: c(7, 8, 9),
                intensity: PerMille::new(900),
                radius: PerMille::new(500),
                offset: PerMille::new(100),
                angle: DeciDegrees::new(2700),
                direction: model::GlowDirection::Both,
            }),
            background: Layer::off(model::TextBox::default()),
            bubble: Layer::on(model::TextBubble {
                spec: MBubbleSpec {
                    kind: model::BubbleKind::ArrowTag {
                        point: model::Side::Left,
                    },
                    tail: Layer::on(model::Tail {
                        tip: TailTip {
                            x: RefPx::from_px(-100),
                            y: RefPx::from_hundredths(25_050),
                        },
                        base: PerMille::new(300),
                        bend: SignedPerMille::new(-500),
                    }),
                    spikes: 20,
                    spike_depth: PerMille::new(250),
                    jitter: PerMille::new(600),
                    seed: 77,
                    lobe: PerMille::new(300),
                    trail: 4,
                },
                fill: c(10, 11, 12),
                opacity: PerMille::new(800),
                pad_x: PerMille::new(600),
                pad_y: PerMille::new(200),
                roundness: PerMille::new(1000),
                border: Layer::on(model::Border {
                    color: c(13, 14, 15),
                    width: PerMille::new(100),
                }),
            }),
            curve: Layer::on(SignedPerMille::new(-250)),
            ..TextStyle::default()
        }
    }

    #[test]
    fn every_style_field_and_layer_is_mapped() {
        let data = TextClipData {
            content: RichText::new("straße"),
            style: styled(),
            layout: TextLayout {
                justify: true,
                box_width: Some(RefPx::from_px(500)),
                ..TextLayout::default()
            },
            ..TextClipData::default()
        };
        let spec = TextSpec::from_clip(&data, 0.5);
        let c = |r, g, b| [r, g, b, 255];
        assert_eq!(spec.text, "STRASSE", "case is applied before layout");
        assert_eq!(spec.align, TextAlign::Justify, "justify wins");
        assert_eq!(spec.box_width_px, Some(250.0));
        assert_eq!(spec.letter_spacing, -0.15);
        assert_eq!(
            spec.decoration,
            Decoration {
                underline: true,
                strike: true
            }
        );
        assert_eq!(
            spec.stroke,
            Some(Stroke {
                color: c(1, 2, 3),
                width: 0.12,
                opacity: 0.75,
                position: StrokePosition::Center,
            })
        );
        assert_eq!(
            spec.shadow,
            Some(Shadow {
                color: c(4, 5, 6),
                opacity: 0.25,
                blur: 0.3,
                distance: 0.4,
                angle_deg: 135.5,
                include_box: true,
            })
        );
        assert_eq!(
            spec.glow,
            Some(Glow {
                color: c(7, 8, 9),
                intensity: 0.9,
                radius: 0.5,
                offset: 0.1,
                angle_deg: 270.0,
                direction: GlowDirection::Both,
            })
        );
        assert_eq!(spec.background, None);
        assert_eq!(
            spec.bubble,
            Some(TextBubble {
                spec: BubbleSpec {
                    kind: BubbleKind::ArrowTag { point: Side::Left },
                    // Ref px times scale_px: layout px from the box centre.
                    tail: Some(Tail {
                        tip: Point::new(-50.0, 125.25),
                        base: 0.3,
                        bend: -0.5,
                    }),
                    spikes: 20,
                    spike_depth: 0.25,
                    jitter: 0.6,
                    seed: 77,
                    lobe: 0.3,
                    trail: 4,
                },
                fill: c(10, 11, 12),
                opacity: 0.8,
                pad_x: 0.6,
                pad_y: 0.2,
                roundness: 1.0,
                border: Some(Border {
                    color: c(13, 14, 15),
                    width: 0.1,
                }),
            })
        );
        assert_eq!(spec.curve, -0.25);
    }

    #[test]
    fn switched_off_layers_become_none() {
        let mut style = styled();
        style.stroke.on = false;
        style.shadow.on = false;
        style.glow.on = false;
        style.bubble.on = false;
        style.curve.on = false;
        style.background = Layer::on(model::TextBox {
            shape: model::BoxShape::PerChar,
            height: model::BoxHeight::LowerHalf,
            ..model::TextBox::default()
        });
        let data = TextClipData {
            style,
            ..TextClipData::default()
        };
        let spec = TextSpec::from_clip(&data, 1.0);
        assert_eq!((spec.stroke, spec.shadow, spec.glow), (None, None, None));
        assert_eq!((spec.bubble, spec.curve), (None, 0.0));
        assert_eq!(
            spec.background,
            Some(TextBox {
                shape: BoxShape::PerChar,
                height: BoxHeight::LowerHalf,
                // The default box border is off.
                border: None,
                ..TextBox::default()
            })
        );
    }

    #[test]
    fn a_tail_that_is_off_and_a_curve_of_zero_draw_neither() {
        let mut style = styled();
        style.bubble.value.spec.tail.on = false;
        style.bubble.value.border.on = false;
        style.curve = Layer::on(SignedPerMille::ZERO);
        let data = TextClipData {
            style,
            ..TextClipData::default()
        };
        let spec = TextSpec::from_clip(&data, 1.0);
        let bubble = spec.bubble.expect("the bubble is on");
        assert_eq!((bubble.spec.tail, bubble.border), (None, None));
        assert!(!spec.is_curved());
    }

    #[test]
    fn every_case_transform_is_applied_without_a_language() {
        let text = |case| {
            let data = TextClipData {
                content: RichText::new("hello wORLD. again\r\nnew"),
                style: TextStyle {
                    case,
                    ..TextStyle::default()
                },
                ..TextClipData::default()
            };
            TextSpec::from_clip(&data, 1.0).text
        };
        assert_eq!(text(CaseTransform::AsTyped), "hello wORLD. again\nnew");
        assert_eq!(text(CaseTransform::Upper), "HELLO WORLD. AGAIN\nNEW");
        assert_eq!(text(CaseTransform::Lower), "hello world. again\nnew");
        assert_eq!(text(CaseTransform::Title), "Hello World. Again\nNew");
        for case in [
            CaseTransform::AsTyped,
            CaseTransform::Upper,
            CaseTransform::Lower,
            CaseTransform::Title,
            CaseTransform::Sentence,
        ] {
            let want = case::display("hello wORLD. again\nnew", case, None::<&LangTag>);
            assert_eq!(text(case), want, "{case:?}");
        }
    }

    #[test]
    fn the_vertical_anchor_is_not_part_of_the_spec() {
        let at = |v_anchor| {
            let data = TextClipData {
                layout: TextLayout {
                    v_anchor,
                    ..TextLayout::default()
                },
                ..TextClipData::default()
            };
            TextSpec::from_clip(&data, 1.0)
        };
        assert_eq!(at(VAnchor::Top), at(VAnchor::Bottom));
        assert_eq!(at(VAnchor::Top), at(VAnchor::Middle));
    }
}
