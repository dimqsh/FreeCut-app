//! Text clips (TX-001 to TX-016, TX-054, TX-055): DTOs ⇄ `TextClipData`.
//!
//! Reading checks shapes and refuses what can't be a value: a colour that
//! isn't `"#RRGGBBAA"`, a gradient without 2 to 8 stops, an unknown kind.
//! Numbers are only brought into range: beyond the model's integer width
//! they are saturated, an angle outside one turn is wrapped, gradient stops
//! past 100 % or out of order are clamped and sorted. Each of these sets
//! `clamped`; the caller then clamps the whole payload with
//! `TextClipData::clamped` (which also lets a bubble win over a background
//! box switched on beside it, TX-054) and reports the clip as repaired.
//!
//! Writing leaves each optional layer out while it is the factory default
//! and carries no unknown keys (see `TextStyleDto`).

use freecut_timeline::model::{
    Border, BoxHeight, BoxShape, BubbleKind, BubbleSpec, CaseTransform, DeciDegrees, Decoration,
    Fill, FillCenter, FillExtent, FontRef, Glow, GlowDirection, GradientStop, GradientStops,
    HAlign, Layer, LinearGradient, PerMille, RadialGradient, RefPx, Rgba8, RichText, Shadow, Side,
    SignedPerMille, Stroke, StrokePosition, Tail, TailTip, TextBox, TextBubble, TextClipData,
    TextLayout, TextOrigin, TextStyle, VAnchor,
};

use super::{At, Give, Variant, color_text, decode_at, decode_tagged, fit, parse_color};
use crate::extras::{Extras, Fields, Owner};
use crate::format::dto::{
    ArrowTagDto, BackgroundDto, BorderDto, BoxHeightDto, BoxShapeDto, BubbleDto, BubbleKindDto,
    CaseDto, CurveDto, FillDto, FillExtentDto, FontDto, GlowDirectionDto, GlowDto, GradientStopDto,
    HAlignDto, LinearFillDto, OutlineDto, PlainDto, PointDto, RadialFillDto, ShadowDto, SideDto,
    SolidFillDto, StrokePositionDto, Tagged, TailDto, TextContentDto, TextLayoutDto, TextOriginDto,
    TextStyleDto, VAnchorDto,
};
use crate::open::OpenError;

/// The reader's state for one clip: where unknown keys go, and whether any
/// value had to be brought into range.
struct Reader<'a> {
    owner: Owner,
    extras: &'a mut Extras,
    clamped: &'a mut bool,
}

impl Reader<'_> {
    /// Files the unknown keys of the object at `path` inside the clip.
    fn file(&mut self, path: &str, fields: Fields) {
        self.extras.put(self.owner, path, fields);
    }

    fn per_mille(&mut self, v: i64) -> PerMille {
        PerMille::new(fit(v, 0, u16::MAX, self.clamped))
    }

    fn signed(&mut self, v: i64) -> SignedPerMille {
        SignedPerMille::new(fit(v, i16::MIN, i16::MAX, self.clamped))
    }

    fn hundredths(&mut self, v: i64) -> RefPx {
        RefPx::from_hundredths(fit(v, i32::MIN, i32::MAX, self.clamped))
    }

    /// Tenths of a degree, wrapped into one turn like the model does.
    fn angle(&mut self, v: i64) -> DeciDegrees {
        if !(0..3600).contains(&v) {
            *self.clamped = true;
        }
        DeciDegrees::new(fit(v, i32::MIN, i32::MAX, self.clamped))
    }
}

/// A text clip's payload, as stored; the caller clamps it.
pub(super) fn read_text(
    dto: TextContentDto,
    at: &At,
    owner: Owner,
    extras: &mut Extras,
    clamped: &mut bool,
) -> Result<TextClipData, OpenError> {
    let mut r = Reader {
        owner,
        extras,
        clamped,
    };
    let style = read_style(dto.style, &at.key("style"), &mut r)?;
    let layout = read_layout(dto.layout, &mut r);
    r.file("content", dto.extra);
    Ok(TextClipData {
        content: RichText::new(dto.text),
        style,
        layout,
        origin: match dto.origin {
            TextOriginDto::Manual => TextOrigin::Manual,
        },
    })
}

fn color(text: &str, at: &At) -> Result<Rgba8, OpenError> {
    parse_color(text)
        .ok_or_else(|| at.bad(format!("expected a color \"#RRGGBBAA\", found {text:?}")))
}

fn read_style(dto: TextStyleDto, at: &At, r: &mut Reader<'_>) -> Result<TextStyle, OpenError> {
    let factory = TextStyle::default();
    let fill = read_fill(dto.fill, &at.key("fill"), r)?;
    let stroke = match dto.outline {
        Some(d) => read_outline(d, &at.key("outline"), r)?,
        None => factory.stroke,
    };
    let shadow = match dto.shadow {
        Some(d) => read_shadow(d, &at.key("shadow"), r)?,
        None => factory.shadow,
    };
    let glow = match dto.glow {
        Some(d) => read_glow(d, &at.key("glow"), r)?,
        None => factory.glow,
    };
    let background = match dto.background {
        Some(d) => read_background(d, &at.key("background"), r)?,
        None => factory.background,
    };
    let bubble = match dto.bubble {
        Some(d) => read_bubble(d, &at.key("bubble"), r)?,
        None => factory.bubble,
    };
    let curve = match dto.curve {
        Some(d) => {
            r.file("content.style.curve", d.extra);
            Layer {
                on: d.on,
                value: r.signed(d.amount),
            }
        }
        None => factory.curve,
    };
    r.file("content.style", dto.extra);
    r.file("content.style.font", dto.font.extra);
    Ok(TextStyle {
        font: FontRef::new(dto.font.family),
        size: r.hundredths(dto.size),
        fill,
        bold: dto.bold,
        italic: dto.italic,
        decoration: Decoration {
            underline: dto.underline,
            strike: dto.strike,
        },
        case: match dto.case {
            CaseDto::AsTyped => CaseTransform::AsTyped,
            CaseDto::Upper => CaseTransform::Upper,
            CaseDto::Lower => CaseTransform::Lower,
            CaseDto::Title => CaseTransform::Title,
            CaseDto::Sentence => CaseTransform::Sentence,
        },
        letter_spacing: r.signed(dto.letter_spacing),
        stroke,
        shadow,
        glow,
        background,
        bubble,
        curve,
    })
}

fn read_fill(dto: Tagged<FillDto>, at: &At, r: &mut Reader<'_>) -> Result<Fill, OpenError> {
    let variants: [Variant<FillDto>; 3] = [
        ("solid", |v, at| decode_at(v, at).map(FillDto::Solid)),
        ("linear-gradient", |v, at| {
            decode_at(v, at).map(FillDto::LinearGradient)
        }),
        ("radial-gradient", |v, at| {
            decode_at(v, at).map(FillDto::RadialGradient)
        }),
    ];
    Ok(match decode_tagged(dto, at, "fill", &variants)? {
        FillDto::Solid(d) => {
            r.file("content.style.fill", d.extra);
            Fill::Solid(color(&d.color, &at.key("color"))?)
        }
        FillDto::LinearGradient(d) => {
            let stops = read_stops(d.stops, &at.key("stops"), r)?;
            r.file("content.style.fill", d.extra);
            Fill::LinearGradient(LinearGradient {
                angle: r.angle(d.angle),
                stops,
                extent: extent(d.extent),
            })
        }
        FillDto::RadialGradient(d) => {
            let stops = read_stops(d.stops, &at.key("stops"), r)?;
            r.file("content.style.fill", d.extra);
            r.file("content.style.fill.center", d.center.extra);
            Fill::RadialGradient(RadialGradient {
                center: FillCenter {
                    x: r.per_mille(d.center.x),
                    y: r.per_mille(d.center.y),
                },
                spread: r.per_mille(d.spread),
                stops,
                extent: extent(d.extent),
            })
        }
    })
}

/// 2–8 stops; positions past 100 % are clamped and the list is sorted
/// (both reported as a repair). Unknown keys travel with their stop through
/// the sort and are filed by its index in the sorted list, the order the
/// writer uses, so they never move to another stop.
fn read_stops(
    dtos: Vec<GradientStopDto>,
    at: &At,
    r: &mut Reader<'_>,
) -> Result<GradientStops, OpenError> {
    let mut stops = Vec::with_capacity(dtos.len());
    for (i, d) in dtos.into_iter().enumerate() {
        let color = color(&d.color, &at.index(i).key("color"))?;
        let mut position = r.per_mille(d.at);
        if position > PerMille::ONE {
            *r.clamped = true;
            position = PerMille::ONE;
        }
        let stop = GradientStop {
            at: position,
            color,
        };
        stops.push((stop, d.extra));
    }
    if !stops.is_sorted_by_key(|(s, _)| s.at) {
        *r.clamped = true;
        // Stable, like `GradientStops::new`: equal positions keep their
        // order.
        stops.sort_by_key(|(s, _)| s.at);
    }
    let mut sorted = Vec::with_capacity(stops.len());
    for (i, (stop, extra)) in stops.into_iter().enumerate() {
        r.file(&format!("content.style.fill.stops[{i}]"), extra);
        sorted.push(stop);
    }
    GradientStops::new(sorted).map_err(|e| at.bad(e))
}

fn extent(dto: FillExtentDto) -> FillExtent {
    match dto {
        FillExtentDto::Block => FillExtent::Block,
        FillExtentDto::Line => FillExtent::Line,
    }
}

fn read_outline(d: OutlineDto, at: &At, r: &mut Reader<'_>) -> Result<Layer<Stroke>, OpenError> {
    r.file("content.style.outline", d.extra);
    Ok(Layer {
        on: d.on,
        value: Stroke {
            color: color(&d.color, &at.key("color"))?,
            width: r.per_mille(d.width),
            opacity: r.per_mille(d.opacity),
            position: match d.position {
                StrokePositionDto::Outside => StrokePosition::Outside,
                StrokePositionDto::Center => StrokePosition::Center,
                StrokePositionDto::Inside => StrokePosition::Inside,
            },
        },
    })
}

fn read_shadow(d: ShadowDto, at: &At, r: &mut Reader<'_>) -> Result<Layer<Shadow>, OpenError> {
    r.file("content.style.shadow", d.extra);
    Ok(Layer {
        on: d.on,
        value: Shadow {
            color: color(&d.color, &at.key("color"))?,
            opacity: r.per_mille(d.opacity),
            blur: r.per_mille(d.blur),
            distance: r.per_mille(d.distance),
            angle: r.angle(d.angle),
            include_box: d.include_box,
        },
    })
}

fn read_glow(d: GlowDto, at: &At, r: &mut Reader<'_>) -> Result<Layer<Glow>, OpenError> {
    r.file("content.style.glow", d.extra);
    Ok(Layer {
        on: d.on,
        value: Glow {
            color: color(&d.color, &at.key("color"))?,
            intensity: r.per_mille(d.intensity),
            radius: r.per_mille(d.radius),
            offset: r.per_mille(d.offset),
            angle: r.angle(d.angle),
            direction: match d.direction {
                GlowDirectionDto::Outward => GlowDirection::Outward,
                GlowDirectionDto::Inward => GlowDirection::Inward,
                GlowDirectionDto::Both => GlowDirection::Both,
            },
        },
    })
}

fn read_border(
    d: BorderDto,
    at: &At,
    path: &str,
    r: &mut Reader<'_>,
) -> Result<Layer<Border>, OpenError> {
    r.file(path, d.extra);
    Ok(Layer {
        on: d.on,
        value: Border {
            color: color(&d.color, &at.key("color"))?,
            width: r.per_mille(d.width),
        },
    })
}

fn read_background(
    d: BackgroundDto,
    at: &At,
    r: &mut Reader<'_>,
) -> Result<Layer<TextBox>, OpenError> {
    r.file("content.style.background", d.extra);
    let border = read_border(
        d.border,
        &at.key("border"),
        "content.style.background.border",
        r,
    )?;
    Ok(Layer {
        on: d.on,
        value: TextBox {
            color: color(&d.color, &at.key("color"))?,
            opacity: r.per_mille(d.opacity),
            shape: match d.shape {
                BoxShapeDto::Block => BoxShape::Block,
                BoxShapeDto::PerLine => BoxShape::PerLine,
                BoxShapeDto::PerChar => BoxShape::PerChar,
            },
            pad_x: r.per_mille(d.pad_x),
            pad_y: r.per_mille(d.pad_y),
            roundness: r.per_mille(d.roundness),
            height: match d.height {
                BoxHeightDto::Full => BoxHeight::Full,
                BoxHeightDto::LowerHalf => BoxHeight::LowerHalf,
            },
            border,
        },
    })
}

fn read_bubble(d: BubbleDto, at: &At, r: &mut Reader<'_>) -> Result<Layer<TextBubble>, OpenError> {
    let variants: [Variant<BubbleKindDto>; 8] = [
        ("rounded", |v, at| {
            decode_at(v, at).map(BubbleKindDto::Rounded)
        }),
        ("oval", |v, at| decode_at(v, at).map(BubbleKindDto::Oval)),
        ("thought", |v, at| {
            decode_at(v, at).map(BubbleKindDto::Thought)
        }),
        ("shout", |v, at| decode_at(v, at).map(BubbleKindDto::Shout)),
        ("square", |v, at| {
            decode_at(v, at).map(BubbleKindDto::Square)
        }),
        ("tag", |v, at| decode_at(v, at).map(BubbleKindDto::Tag)),
        ("arrow-tag", |v, at| {
            decode_at(v, at).map(BubbleKindDto::ArrowTag)
        }),
        ("ticket", |v, at| {
            decode_at(v, at).map(BubbleKindDto::Ticket)
        }),
    ];
    let (kind, kind_extra) = match decode_tagged(d.kind, &at.key("kind"), "bubble kind", &variants)?
    {
        BubbleKindDto::Rounded(p) => (BubbleKind::Rounded, p.extra),
        BubbleKindDto::Oval(p) => (BubbleKind::Oval, p.extra),
        BubbleKindDto::Thought(p) => (BubbleKind::Thought, p.extra),
        BubbleKindDto::Shout(p) => (BubbleKind::Shout, p.extra),
        BubbleKindDto::Square(p) => (BubbleKind::Square, p.extra),
        BubbleKindDto::Tag(p) => (BubbleKind::Tag, p.extra),
        BubbleKindDto::ArrowTag(a) => (
            BubbleKind::ArrowTag {
                point: match a.point {
                    SideDto::Left => Side::Left,
                    SideDto::Right => Side::Right,
                },
            },
            a.extra,
        ),
        BubbleKindDto::Ticket(p) => (BubbleKind::Ticket, p.extra),
    };
    r.file("content.style.bubble", d.extra);
    r.file("content.style.bubble.kind", kind_extra);
    r.file("content.style.bubble.tail", d.tail.extra);
    r.file("content.style.bubble.tail.tip", d.tail.tip.extra);
    let border = read_border(
        d.border,
        &at.key("border"),
        "content.style.bubble.border",
        r,
    )?;
    let tail = Layer {
        on: d.tail.on,
        value: Tail {
            tip: TailTip {
                x: r.hundredths(d.tail.tip.x),
                y: r.hundredths(d.tail.tip.y),
            },
            base: r.per_mille(d.tail.base),
            bend: r.signed(d.tail.bend),
        },
    };
    Ok(Layer {
        on: d.on,
        value: TextBubble {
            spec: BubbleSpec {
                kind,
                tail,
                spikes: fit(d.spikes, 0, u8::MAX, r.clamped),
                spike_depth: r.per_mille(d.spike_depth),
                jitter: r.per_mille(d.jitter),
                seed: fit(d.seed, 0, u32::MAX, r.clamped),
                lobe: r.per_mille(d.lobe),
                trail: fit(d.trail, 0, u8::MAX, r.clamped),
            },
            fill: color(&d.color, &at.key("color"))?,
            opacity: r.per_mille(d.opacity),
            pad_x: r.per_mille(d.pad_x),
            pad_y: r.per_mille(d.pad_y),
            roundness: r.per_mille(d.roundness),
            border,
        },
    })
}

fn read_layout(d: TextLayoutDto, r: &mut Reader<'_>) -> TextLayout {
    r.file("content.layout", d.extra);
    TextLayout {
        align: match d.align {
            HAlignDto::Left => HAlign::Left,
            HAlignDto::Center => HAlign::Center,
            HAlignDto::Right => HAlign::Right,
        },
        justify: d.justify,
        box_width: d.box_width.map(|w| r.hundredths(w)),
        line_spacing: r.per_mille(d.line_spacing),
        v_anchor: match d.v_anchor {
            VAnchorDto::Top => VAnchor::Top,
            VAnchorDto::Middle => VAnchor::Middle,
            VAnchorDto::Bottom => VAnchor::Bottom,
        },
    }
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

pub(super) fn write_text(data: &TextClipData, give: &Give<'_>) -> TextContentDto {
    let layout = &data.layout;
    TextContentDto {
        text: data.content.as_str().to_string(),
        style: write_style(&data.style, give),
        layout: TextLayoutDto {
            align: match layout.align {
                HAlign::Left => HAlignDto::Left,
                HAlign::Center => HAlignDto::Center,
                HAlign::Right => HAlignDto::Right,
            },
            justify: layout.justify,
            box_width: layout.box_width.map(|w| i64::from(w.hundredths())),
            line_spacing: i64::from(layout.line_spacing.get()),
            v_anchor: match layout.v_anchor {
                VAnchor::Top => VAnchorDto::Top,
                VAnchor::Middle => VAnchorDto::Middle,
                VAnchor::Bottom => VAnchorDto::Bottom,
            },
            extra: give.at("content.layout"),
        },
        // `check_writable` refuses any other origin before writing.
        origin: TextOriginDto::Manual,
        extra: Fields::new(),
    }
}

/// Whether a layer is written: it differs from the factory default, or
/// one of its objects carries unknown keys.
fn written<T: PartialEq>(layer: &Layer<T>, factory: &Layer<T>, extras: &[&Fields]) -> bool {
    layer != factory || extras.iter().any(|f| !f.is_empty())
}

fn pm(p: PerMille) -> i64 {
    i64::from(p.get())
}

fn spm(p: SignedPerMille) -> i64 {
    i64::from(p.get())
}

fn deg(a: DeciDegrees) -> i64 {
    i64::from(a.tenths())
}

fn write_style(style: &TextStyle, give: &Give<'_>) -> TextStyleDto {
    let factory = TextStyle::default();
    TextStyleDto {
        font: FontDto {
            family: style.font.family.clone(),
            extra: give.at("content.style.font"),
        },
        size: i64::from(style.size.hundredths()),
        fill: Tagged::Decoded(write_fill(&style.fill, give)),
        bold: style.bold,
        italic: style.italic,
        underline: style.decoration.underline,
        strike: style.decoration.strike,
        case: match style.case {
            CaseTransform::AsTyped => CaseDto::AsTyped,
            CaseTransform::Upper => CaseDto::Upper,
            CaseTransform::Lower => CaseDto::Lower,
            CaseTransform::Title => CaseDto::Title,
            CaseTransform::Sentence => CaseDto::Sentence,
        },
        letter_spacing: spm(style.letter_spacing),
        outline: write_outline(&style.stroke, &factory.stroke, give),
        shadow: write_shadow(&style.shadow, &factory.shadow, give),
        glow: write_glow(&style.glow, &factory.glow, give),
        background: write_background(&style.background, &factory.background, give),
        bubble: write_bubble(&style.bubble, &factory.bubble, give),
        curve: {
            let extra = give.at("content.style.curve");
            written(&style.curve, &factory.curve, &[&extra]).then(|| CurveDto {
                on: style.curve.on,
                amount: spm(style.curve.value),
                extra,
            })
        },
        extra: give.at("content.style"),
    }
}

fn write_fill(fill: &Fill, give: &Give<'_>) -> FillDto {
    const AT: &str = "content.style.fill";
    let stops = |stops: &GradientStops| -> Vec<GradientStopDto> {
        stops
            .as_slice()
            .iter()
            .enumerate()
            .map(|(i, s)| GradientStopDto {
                at: pm(s.at),
                color: color_text(s.color),
                extra: give.at_with(|| format!("{AT}.stops[{i}]")),
            })
            .collect()
    };
    let extent = |e: FillExtent| match e {
        FillExtent::Block => FillExtentDto::Block,
        FillExtent::Line => FillExtentDto::Line,
    };
    match fill {
        Fill::Solid(c) => FillDto::Solid(give.variant(
            AT,
            SolidFillDto {
                color: color_text(*c),
                extra: Fields::new(),
            },
            |d| &mut d.extra,
        )),
        Fill::LinearGradient(g) => FillDto::LinearGradient(give.variant(
            AT,
            LinearFillDto {
                angle: deg(g.angle),
                stops: stops(&g.stops),
                extent: extent(g.extent),
                extra: Fields::new(),
            },
            |d| &mut d.extra,
        )),
        Fill::RadialGradient(g) => FillDto::RadialGradient(give.variant(
            AT,
            RadialFillDto {
                center: PointDto {
                    x: pm(g.center.x),
                    y: pm(g.center.y),
                    extra: give.at("content.style.fill.center"),
                },
                spread: pm(g.spread),
                stops: stops(&g.stops),
                extent: extent(g.extent),
                extra: Fields::new(),
            },
            |d| &mut d.extra,
        )),
    }
}

fn write_outline(
    layer: &Layer<Stroke>,
    factory: &Layer<Stroke>,
    give: &Give<'_>,
) -> Option<OutlineDto> {
    let extra = give.at("content.style.outline");
    written(layer, factory, &[&extra]).then(|| {
        let s = &layer.value;
        OutlineDto {
            on: layer.on,
            color: color_text(s.color),
            width: pm(s.width),
            opacity: pm(s.opacity),
            position: match s.position {
                StrokePosition::Outside => StrokePositionDto::Outside,
                StrokePosition::Center => StrokePositionDto::Center,
                StrokePosition::Inside => StrokePositionDto::Inside,
            },
            extra,
        }
    })
}

fn write_shadow(
    layer: &Layer<Shadow>,
    factory: &Layer<Shadow>,
    give: &Give<'_>,
) -> Option<ShadowDto> {
    let extra = give.at("content.style.shadow");
    written(layer, factory, &[&extra]).then(|| {
        let s = &layer.value;
        ShadowDto {
            on: layer.on,
            color: color_text(s.color),
            opacity: pm(s.opacity),
            blur: pm(s.blur),
            distance: pm(s.distance),
            angle: deg(s.angle),
            include_box: s.include_box,
            extra,
        }
    })
}

fn write_glow(layer: &Layer<Glow>, factory: &Layer<Glow>, give: &Give<'_>) -> Option<GlowDto> {
    let extra = give.at("content.style.glow");
    written(layer, factory, &[&extra]).then(|| {
        let g = &layer.value;
        GlowDto {
            on: layer.on,
            color: color_text(g.color),
            intensity: pm(g.intensity),
            radius: pm(g.radius),
            offset: pm(g.offset),
            angle: deg(g.angle),
            direction: match g.direction {
                GlowDirection::Outward => GlowDirectionDto::Outward,
                GlowDirection::Inward => GlowDirectionDto::Inward,
                GlowDirection::Both => GlowDirectionDto::Both,
            },
            extra,
        }
    })
}

fn write_border(border: &Layer<Border>, extra: Fields) -> BorderDto {
    BorderDto {
        on: border.on,
        color: color_text(border.value.color),
        width: pm(border.value.width),
        extra,
    }
}

fn write_background(
    layer: &Layer<TextBox>,
    factory: &Layer<TextBox>,
    give: &Give<'_>,
) -> Option<BackgroundDto> {
    let extra = give.at("content.style.background");
    let border_extra = give.at("content.style.background.border");
    written(layer, factory, &[&extra, &border_extra]).then(|| {
        let b = &layer.value;
        BackgroundDto {
            on: layer.on,
            color: color_text(b.color),
            opacity: pm(b.opacity),
            shape: match b.shape {
                BoxShape::Block => BoxShapeDto::Block,
                BoxShape::PerLine => BoxShapeDto::PerLine,
                BoxShape::PerChar => BoxShapeDto::PerChar,
            },
            pad_x: pm(b.pad_x),
            pad_y: pm(b.pad_y),
            roundness: pm(b.roundness),
            height: match b.height {
                BoxHeight::Full => BoxHeightDto::Full,
                BoxHeight::LowerHalf => BoxHeightDto::LowerHalf,
            },
            border: write_border(&b.border, border_extra),
            extra,
        }
    })
}

fn write_bubble(
    layer: &Layer<TextBubble>,
    factory: &Layer<TextBubble>,
    give: &Give<'_>,
) -> Option<BubbleDto> {
    const AT: &str = "content.style.bubble";
    let extra = give.at(AT);
    let kind_extra = give.at("content.style.bubble.kind");
    let tail_extra = give.at("content.style.bubble.tail");
    let tip_extra = give.at("content.style.bubble.tail.tip");
    let border_extra = give.at("content.style.bubble.border");
    let all = [&extra, &kind_extra, &tail_extra, &tip_extra, &border_extra];
    if !written(layer, factory, &all) {
        return None;
    }
    let b = &layer.value;
    let spec = &b.spec;
    let plain = || {
        give.variant(
            "content.style.bubble.kind",
            PlainDto {
                extra: Fields::new(),
            },
            |d| &mut d.extra,
        )
    };
    let kind = match spec.kind {
        BubbleKind::Rounded => BubbleKindDto::Rounded(plain()),
        BubbleKind::Oval => BubbleKindDto::Oval(plain()),
        BubbleKind::Thought => BubbleKindDto::Thought(plain()),
        BubbleKind::Shout => BubbleKindDto::Shout(plain()),
        BubbleKind::Square => BubbleKindDto::Square(plain()),
        BubbleKind::Tag => BubbleKindDto::Tag(plain()),
        BubbleKind::ArrowTag { point } => BubbleKindDto::ArrowTag(give.variant(
            "content.style.bubble.kind",
            ArrowTagDto {
                point: match point {
                    Side::Left => SideDto::Left,
                    Side::Right => SideDto::Right,
                },
                extra: Fields::new(),
            },
            |d| &mut d.extra,
        )),
        BubbleKind::Ticket => BubbleKindDto::Ticket(plain()),
    };
    let tail = &spec.tail;
    Some(BubbleDto {
        on: layer.on,
        kind: Tagged::Decoded(kind),
        tail: TailDto {
            on: tail.on,
            tip: PointDto {
                x: i64::from(tail.value.tip.x.hundredths()),
                y: i64::from(tail.value.tip.y.hundredths()),
                extra: tip_extra,
            },
            base: pm(tail.value.base),
            bend: spm(tail.value.bend),
            extra: tail_extra,
        },
        color: color_text(b.fill),
        opacity: pm(b.opacity),
        pad_x: pm(b.pad_x),
        pad_y: pm(b.pad_y),
        roundness: pm(b.roundness),
        border: write_border(&b.border, border_extra),
        spikes: i64::from(spec.spikes),
        spike_depth: pm(spec.spike_depth),
        jitter: pm(spec.jitter),
        seed: i64::from(spec.seed),
        lobe: pm(spec.lobe),
        trail: i64::from(spec.trail),
        extra,
    })
}
