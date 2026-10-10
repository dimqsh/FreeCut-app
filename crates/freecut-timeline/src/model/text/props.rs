//! Text properties by name (TX-021, TX-054): a closed list of property
//! paths with stable strings, the inspector cards that hold them, and the
//! lenses that hold the cards. Pure tables shared by card reset, the card
//! keyframe commands, the command bar ("Go to setting") and preset files.

use super::TextClipData;
use super::style::Fill;

/// The cards of the text inspector (TX-054).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TextCard {
    Text,
    Paragraph,
    Type,
    Fill,
    Spacing,
    Outline,
    Glow,
    Shadow,
    Background,
    Bubble,
    Curve,
}

impl TextCard {
    pub const ALL: [TextCard; 11] = [
        TextCard::Text,
        TextCard::Paragraph,
        TextCard::Type,
        TextCard::Fill,
        TextCard::Spacing,
        TextCard::Outline,
        TextCard::Glow,
        TextCard::Shadow,
        TextCard::Background,
        TextCard::Bubble,
        TextCard::Curve,
    ];

    /// The lens showing this card (TX-054).
    pub const fn lens(self) -> TextLens {
        match self {
            TextCard::Text | TextCard::Paragraph => TextLens::Content,
            TextCard::Type
            | TextCard::Fill
            | TextCard::Spacing
            | TextCard::Outline
            | TextCard::Glow
            | TextCard::Shadow => TextLens::Style,
            TextCard::Background | TextCard::Bubble | TextCard::Curve => TextLens::BoxAndBubble,
        }
    }
}

/// The text lenses that hold text properties (TX-054). Motion's keyframes
/// and animation slots arrive with the animation engine; Presets has no
/// properties of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextLens {
    Content,
    Style,
    /// Its Placement card is the clip transform (TR-001), not a text card.
    BoxAndBubble,
}

impl TextLens {
    /// The text cards of this lens, top to bottom.
    pub fn cards(self) -> &'static [TextCard] {
        match self {
            TextLens::Content => &[TextCard::Text, TextCard::Paragraph],
            TextLens::Style => &[
                TextCard::Type,
                TextCard::Fill,
                TextCard::Spacing,
                TextCard::Outline,
                TextCard::Glow,
                TextCard::Shadow,
            ],
            TextLens::BoxAndBubble => &[TextCard::Background, TextCard::Bubble, TextCard::Curve],
        }
    }
}

macro_rules! text_props {
    ($($variant:ident => $path:literal, $card:ident;)*) => {
        /// A text property (TX-054), with a stable string form such as
        /// `text.outline.width`. The strings are part of preset files and
        /// the command bar: never change one, only add.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum TextPropPath {
            $($variant,)*
        }

        impl TextPropPath {
            pub const ALL: &'static [TextPropPath] = &[$(TextPropPath::$variant,)*];

            /// The stable string form.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(TextPropPath::$variant => $path,)*
                }
            }

            /// The card holding this property.
            pub const fn card(self) -> TextCard {
                match self {
                    $(TextPropPath::$variant => TextCard::$card,)*
                }
            }
        }
    };
}

text_props! {
    Content => "text.content", Text;
    Align => "text.align", Paragraph;
    Justify => "text.justify", Paragraph;
    BoxWidth => "text.box_width", Paragraph;
    VAnchor => "text.v_anchor", Paragraph;
    Font => "text.font", Type;
    Size => "text.size", Type;
    Bold => "text.bold", Type;
    Italic => "text.italic", Type;
    Underline => "text.underline", Type;
    Strike => "text.strike", Type;
    Case => "text.case", Type;
    FillKind => "text.fill.kind", Fill;
    FillColor => "text.fill.color", Fill;
    FillStops => "text.fill.stops", Fill;
    FillAngle => "text.fill.angle", Fill;
    FillCenter => "text.fill.center", Fill;
    FillSpread => "text.fill.spread", Fill;
    FillExtent => "text.fill.extent", Fill;
    LetterSpacing => "text.letter_spacing", Spacing;
    LineSpacing => "text.line_spacing", Spacing;
    OutlineOn => "text.outline.on", Outline;
    OutlineColor => "text.outline.color", Outline;
    OutlineWidth => "text.outline.width", Outline;
    OutlineOpacity => "text.outline.opacity", Outline;
    OutlinePosition => "text.outline.position", Outline;
    GlowOn => "text.glow.on", Glow;
    GlowColor => "text.glow.color", Glow;
    GlowIntensity => "text.glow.intensity", Glow;
    GlowRadius => "text.glow.radius", Glow;
    GlowOffset => "text.glow.offset", Glow;
    GlowAngle => "text.glow.angle", Glow;
    GlowDirection => "text.glow.direction", Glow;
    ShadowOn => "text.shadow.on", Shadow;
    ShadowColor => "text.shadow.color", Shadow;
    ShadowOpacity => "text.shadow.opacity", Shadow;
    ShadowBlur => "text.shadow.blur", Shadow;
    ShadowDistance => "text.shadow.distance", Shadow;
    ShadowAngle => "text.shadow.angle", Shadow;
    ShadowIncludeBox => "text.shadow.include_box", Shadow;
    BackgroundOn => "text.background.on", Background;
    BackgroundColor => "text.background.color", Background;
    BackgroundOpacity => "text.background.opacity", Background;
    BackgroundShape => "text.background.shape", Background;
    BackgroundPadX => "text.background.pad_x", Background;
    BackgroundPadY => "text.background.pad_y", Background;
    BackgroundRoundness => "text.background.roundness", Background;
    BackgroundHeight => "text.background.height", Background;
    BackgroundBorderOn => "text.background.border.on", Background;
    BackgroundBorderColor => "text.background.border.color", Background;
    BackgroundBorderWidth => "text.background.border.width", Background;
    BubbleOn => "text.bubble.on", Bubble;
    BubbleKind => "text.bubble.kind", Bubble;
    BubbleTailOn => "text.bubble.tail.on", Bubble;
    BubbleColor => "text.bubble.color", Bubble;
    BubbleOpacity => "text.bubble.opacity", Bubble;
    BubbleTailTip => "text.bubble.tail.tip", Bubble;
    BubbleTailBase => "text.bubble.tail.base", Bubble;
    BubbleTailBend => "text.bubble.tail.bend", Bubble;
    BubblePadX => "text.bubble.pad_x", Bubble;
    BubblePadY => "text.bubble.pad_y", Bubble;
    BubbleRoundness => "text.bubble.roundness", Bubble;
    BubbleBorderOn => "text.bubble.border.on", Bubble;
    BubbleBorderColor => "text.bubble.border.color", Bubble;
    BubbleBorderWidth => "text.bubble.border.width", Bubble;
    BubbleSpikes => "text.bubble.spikes", Bubble;
    BubbleSpikeDepth => "text.bubble.spike_depth", Bubble;
    BubbleJitter => "text.bubble.jitter", Bubble;
    BubbleSeed => "text.bubble.seed", Bubble;
    BubbleLobe => "text.bubble.lobe", Bubble;
    BubbleTrail => "text.bubble.trail", Bubble;
    CurveOn => "text.curve.on", Curve;
    CurveAmount => "text.curve.amount", Curve;
}

impl TextPropPath {
    /// The property named `path`, if it is one.
    pub fn parse(path: &str) -> Option<TextPropPath> {
        Self::ALL.iter().copied().find(|p| p.as_str() == path)
    }

    /// The on/off switch of an optional layer (TX-054): kept by card and
    /// lens reset.
    pub const fn is_layer_switch(self) -> bool {
        matches!(
            self,
            TextPropPath::OutlineOn
                | TextPropPath::GlowOn
                | TextPropPath::ShadowOn
                | TextPropPath::BackgroundOn
                | TextPropPath::BubbleOn
                | TextPropPath::CurveOn
        )
    }

    /// Whether TX-021 lets this property take keyframes. Gradient stop
    /// colours are keyed stop by stop and get their own addressing with
    /// the keyframe engine.
    pub const fn is_animatable(self) -> bool {
        use TextPropPath as P;
        matches!(
            self,
            P::Size
                | P::FillColor
                | P::FillAngle
                | P::LetterSpacing
                | P::LineSpacing
                | P::OutlineColor
                | P::OutlineWidth
                | P::OutlineOpacity
                | P::GlowColor
                | P::GlowIntensity
                | P::GlowRadius
                | P::GlowOffset
                | P::ShadowColor
                | P::ShadowOpacity
                | P::ShadowBlur
                | P::ShadowDistance
                | P::ShadowAngle
                | P::BackgroundColor
                | P::BackgroundOpacity
                | P::BackgroundRoundness
                | P::BackgroundBorderWidth
                | P::BubbleTailTip
                | P::BubbleTailBase
                | P::BubbleTailBend
                | P::CurveAmount
        )
    }

    /// Copies this property's value from `from` into `to`, leaving every
    /// other property alone. `FillKind` carries the whole fill (kind, stops
    /// and geometry); the other fill paths copy their part only when both
    /// fills have it.
    pub fn copy(self, from: &TextClipData, to: &mut TextClipData) {
        use TextPropPath as P;
        let (fs, ts) = (&from.style, &mut to.style);
        let (fl, tl) = (&from.layout, &mut to.layout);
        let (fb, tb) = (&fs.background.value, &mut ts.background.value);
        let (fu, tu) = (&fs.bubble.value, &mut ts.bubble.value);
        match self {
            P::Content => to.content.clone_from(&from.content),
            P::Align => tl.align = fl.align,
            P::Justify => tl.justify = fl.justify,
            P::BoxWidth => tl.box_width = fl.box_width,
            P::VAnchor => tl.v_anchor = fl.v_anchor,
            P::Font => ts.font.clone_from(&fs.font),
            P::Size => ts.size = fs.size,
            P::Bold => ts.bold = fs.bold,
            P::Italic => ts.italic = fs.italic,
            P::Underline => ts.decoration.underline = fs.decoration.underline,
            P::Strike => ts.decoration.strike = fs.decoration.strike,
            P::Case => ts.case = fs.case,
            P::FillKind => ts.fill.clone_from(&fs.fill),
            P::FillColor => {
                if let (Fill::Solid(f), Fill::Solid(t)) = (&fs.fill, &mut ts.fill) {
                    *t = *f;
                }
            }
            P::FillStops => match (&fs.fill, &mut ts.fill) {
                (Fill::LinearGradient(f), Fill::LinearGradient(t)) => t.stops.clone_from(&f.stops),
                (Fill::RadialGradient(f), Fill::RadialGradient(t)) => t.stops.clone_from(&f.stops),
                (Fill::LinearGradient(f), Fill::RadialGradient(t)) => t.stops.clone_from(&f.stops),
                (Fill::RadialGradient(f), Fill::LinearGradient(t)) => t.stops.clone_from(&f.stops),
                _ => {}
            },
            P::FillAngle => {
                if let (Fill::LinearGradient(f), Fill::LinearGradient(t)) = (&fs.fill, &mut ts.fill)
                {
                    t.angle = f.angle;
                }
            }
            P::FillCenter => {
                if let (Fill::RadialGradient(f), Fill::RadialGradient(t)) = (&fs.fill, &mut ts.fill)
                {
                    t.center = f.center;
                }
            }
            P::FillSpread => {
                if let (Fill::RadialGradient(f), Fill::RadialGradient(t)) = (&fs.fill, &mut ts.fill)
                {
                    t.spread = f.spread;
                }
            }
            P::FillExtent => match (&fs.fill, &mut ts.fill) {
                (Fill::LinearGradient(f), Fill::LinearGradient(t)) => t.extent = f.extent,
                (Fill::RadialGradient(f), Fill::RadialGradient(t)) => t.extent = f.extent,
                (Fill::LinearGradient(f), Fill::RadialGradient(t)) => t.extent = f.extent,
                (Fill::RadialGradient(f), Fill::LinearGradient(t)) => t.extent = f.extent,
                _ => {}
            },
            P::LetterSpacing => ts.letter_spacing = fs.letter_spacing,
            P::LineSpacing => tl.line_spacing = fl.line_spacing,
            P::OutlineOn => ts.stroke.on = fs.stroke.on,
            P::OutlineColor => ts.stroke.value.color = fs.stroke.value.color,
            P::OutlineWidth => ts.stroke.value.width = fs.stroke.value.width,
            P::OutlineOpacity => ts.stroke.value.opacity = fs.stroke.value.opacity,
            P::OutlinePosition => ts.stroke.value.position = fs.stroke.value.position,
            P::GlowOn => ts.glow.on = fs.glow.on,
            P::GlowColor => ts.glow.value.color = fs.glow.value.color,
            P::GlowIntensity => ts.glow.value.intensity = fs.glow.value.intensity,
            P::GlowRadius => ts.glow.value.radius = fs.glow.value.radius,
            P::GlowOffset => ts.glow.value.offset = fs.glow.value.offset,
            P::GlowAngle => ts.glow.value.angle = fs.glow.value.angle,
            P::GlowDirection => ts.glow.value.direction = fs.glow.value.direction,
            P::ShadowOn => ts.shadow.on = fs.shadow.on,
            P::ShadowColor => ts.shadow.value.color = fs.shadow.value.color,
            P::ShadowOpacity => ts.shadow.value.opacity = fs.shadow.value.opacity,
            P::ShadowBlur => ts.shadow.value.blur = fs.shadow.value.blur,
            P::ShadowDistance => ts.shadow.value.distance = fs.shadow.value.distance,
            P::ShadowAngle => ts.shadow.value.angle = fs.shadow.value.angle,
            P::ShadowIncludeBox => ts.shadow.value.include_box = fs.shadow.value.include_box,
            P::BackgroundOn => ts.background.on = fs.background.on,
            P::BackgroundColor => tb.color = fb.color,
            P::BackgroundOpacity => tb.opacity = fb.opacity,
            P::BackgroundShape => tb.shape = fb.shape,
            P::BackgroundPadX => tb.pad_x = fb.pad_x,
            P::BackgroundPadY => tb.pad_y = fb.pad_y,
            P::BackgroundRoundness => tb.roundness = fb.roundness,
            P::BackgroundHeight => tb.height = fb.height,
            P::BackgroundBorderOn => tb.border.on = fb.border.on,
            P::BackgroundBorderColor => tb.border.value.color = fb.border.value.color,
            P::BackgroundBorderWidth => tb.border.value.width = fb.border.value.width,
            P::BubbleOn => ts.bubble.on = fs.bubble.on,
            P::BubbleKind => tu.spec.kind = fu.spec.kind,
            P::BubbleTailOn => tu.spec.tail.on = fu.spec.tail.on,
            P::BubbleColor => tu.fill = fu.fill,
            P::BubbleOpacity => tu.opacity = fu.opacity,
            P::BubbleTailTip => tu.spec.tail.value.tip = fu.spec.tail.value.tip,
            P::BubbleTailBase => tu.spec.tail.value.base = fu.spec.tail.value.base,
            P::BubbleTailBend => tu.spec.tail.value.bend = fu.spec.tail.value.bend,
            P::BubblePadX => tu.pad_x = fu.pad_x,
            P::BubblePadY => tu.pad_y = fu.pad_y,
            P::BubbleRoundness => tu.roundness = fu.roundness,
            P::BubbleBorderOn => tu.border.on = fu.border.on,
            P::BubbleBorderColor => tu.border.value.color = fu.border.value.color,
            P::BubbleBorderWidth => tu.border.value.width = fu.border.value.width,
            P::BubbleSpikes => tu.spec.spikes = fu.spec.spikes,
            P::BubbleSpikeDepth => tu.spec.spike_depth = fu.spec.spike_depth,
            P::BubbleJitter => tu.spec.jitter = fu.spec.jitter,
            P::BubbleSeed => tu.spec.seed = fu.spec.seed,
            P::BubbleLobe => tu.spec.lobe = fu.spec.lobe,
            P::BubbleTrail => tu.spec.trail = fu.spec.trail,
            P::CurveOn => ts.curve.on = fs.curve.on,
            P::CurveAmount => ts.curve.value = fs.curve.value,
        }
    }
}

impl core::fmt::Display for TextPropPath {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The properties a card holds, in the card's row order (TX-021, TX-054).
pub fn card_props(card: TextCard) -> &'static [TextPropPath] {
    use TextPropPath as P;
    match card {
        TextCard::Text => &[P::Content],
        TextCard::Paragraph => &[P::Align, P::Justify, P::BoxWidth, P::VAnchor],
        TextCard::Type => &[
            P::Font,
            P::Size,
            P::Bold,
            P::Italic,
            P::Underline,
            P::Strike,
            P::Case,
        ],
        // The kind first: it carries the whole fill.
        TextCard::Fill => &[
            P::FillKind,
            P::FillColor,
            P::FillStops,
            P::FillAngle,
            P::FillCenter,
            P::FillSpread,
            P::FillExtent,
        ],
        TextCard::Spacing => &[P::LetterSpacing, P::LineSpacing],
        TextCard::Outline => &[
            P::OutlineOn,
            P::OutlineColor,
            P::OutlineWidth,
            P::OutlineOpacity,
            P::OutlinePosition,
        ],
        TextCard::Glow => &[
            P::GlowOn,
            P::GlowColor,
            P::GlowIntensity,
            P::GlowRadius,
            P::GlowOffset,
            P::GlowAngle,
            P::GlowDirection,
        ],
        TextCard::Shadow => &[
            P::ShadowOn,
            P::ShadowColor,
            P::ShadowOpacity,
            P::ShadowBlur,
            P::ShadowDistance,
            P::ShadowAngle,
            P::ShadowIncludeBox,
        ],
        TextCard::Background => &[
            P::BackgroundOn,
            P::BackgroundColor,
            P::BackgroundOpacity,
            P::BackgroundShape,
            P::BackgroundPadX,
            P::BackgroundPadY,
            P::BackgroundRoundness,
            P::BackgroundHeight,
            P::BackgroundBorderOn,
            P::BackgroundBorderColor,
            P::BackgroundBorderWidth,
        ],
        TextCard::Bubble => &[
            P::BubbleOn,
            P::BubbleKind,
            P::BubbleTailOn,
            P::BubbleColor,
            P::BubbleOpacity,
            P::BubbleTailTip,
            P::BubbleTailBase,
            P::BubbleTailBend,
            P::BubblePadX,
            P::BubblePadY,
            P::BubbleRoundness,
            P::BubbleBorderOn,
            P::BubbleBorderColor,
            P::BubbleBorderWidth,
            P::BubbleSpikes,
            P::BubbleSpikeDepth,
            P::BubbleJitter,
            P::BubbleSeed,
            P::BubbleLobe,
            P::BubbleTrail,
        ],
        TextCard::Curve => &[P::CurveOn, P::CurveAmount],
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::model::text::{
        BubbleKind, CaseTransform, FillKind, Layer, RefPx, Rgba8, SignedPerMille, TailTip,
        TextLayerKind,
    };
    use crate::model::{HAlign, PerMille};

    #[test]
    fn paths_are_unique_stable_and_parse_back() {
        let strings: HashSet<&str> = TextPropPath::ALL.iter().map(|p| p.as_str()).collect();
        assert_eq!(strings.len(), TextPropPath::ALL.len());
        for p in TextPropPath::ALL {
            assert_eq!(TextPropPath::parse(p.as_str()), Some(*p));
            assert!(p.as_str().starts_with("text."));
            assert_eq!(p.to_string(), p.as_str());
        }
        assert_eq!(
            TextPropPath::parse("text.outline.width"),
            Some(TextPropPath::OutlineWidth)
        );
        assert_eq!(
            TextPropPath::parse("text.bubble.tail.tip"),
            Some(TextPropPath::BubbleTailTip)
        );
        assert_eq!(TextPropPath::parse("text.nope"), None);
    }

    #[test]
    fn every_path_is_on_exactly_its_own_card() {
        let mut seen = HashSet::new();
        for card in TextCard::ALL {
            for p in card_props(card) {
                assert_eq!(p.card(), card, "{p}");
                assert!(seen.insert(*p), "{p} listed twice");
            }
        }
        assert_eq!(seen.len(), TextPropPath::ALL.len());
        for lens in [TextLens::Content, TextLens::Style, TextLens::BoxAndBubble] {
            for card in lens.cards() {
                assert_eq!(card.lens(), lens);
            }
        }
        let in_lenses: usize = [TextLens::Content, TextLens::Style, TextLens::BoxAndBubble]
            .iter()
            .map(|l| l.cards().len())
            .sum();
        assert_eq!(in_lenses, TextCard::ALL.len());
    }

    #[test]
    fn layer_switches_and_animatable_rows() {
        let switches: Vec<_> = TextPropPath::ALL
            .iter()
            .filter(|p| p.is_layer_switch())
            .collect();
        assert_eq!(switches.len(), 6);
        assert!(!TextPropPath::Content.is_animatable());
        assert!(!TextPropPath::Font.is_animatable());
        assert!(!TextPropPath::OutlineOn.is_animatable());
        assert!(TextPropPath::BubbleTailTip.is_animatable());
    }

    /// Data where every property differs from the factory default.
    fn everything_changed() -> TextClipData {
        let mut d = TextClipData::with_content("changed");
        let s = &mut d.style;
        s.font.family = "Other".into();
        s.size = RefPx::from_px(99);
        s.bold = false;
        s.italic = true;
        s.decoration.underline = true;
        s.decoration.strike = true;
        s.case = CaseTransform::Upper;
        s.fill = Fill::Solid(Rgba8::BLACK).with_kind(FillKind::RadialGradient);
        s.letter_spacing = SignedPerMille::new(50);
        s.stroke = Layer::on(s.stroke.value);
        s.stroke.value.width = PerMille::new(10);
        s.curve = Layer::on(SignedPerMille::new(300));
        s.bubble = s.bubble.map(|mut b| {
            b.spec.kind = BubbleKind::Shout;
            b.spec.tail.value.tip = TailTip {
                x: RefPx::from_px(1),
                y: RefPx::from_px(2),
            };
            b
        });
        d.style = d.style.with_layer(TextLayerKind::Bubble, true);
        d.layout.align = HAlign::Left;
        d.layout.justify = true;
        d.layout.box_width = Some(RefPx::from_px(300));
        d.layout.line_spacing = PerMille::new(1200);
        d
    }

    #[test]
    fn copying_a_card_copies_exactly_its_properties() {
        let base = TextClipData::default();
        let other = everything_changed();
        for card in TextCard::ALL {
            let mut out = base.clone();
            for p in card_props(card) {
                p.copy(&other, &mut out);
            }
            // Copying the card back from base restores base exactly.
            let mut back = out.clone();
            for p in card_props(card) {
                p.copy(&base, &mut back);
            }
            assert_eq!(back, base, "{card:?}");
        }
        // Copying every property gives the other data.
        let mut all = base.clone();
        for p in TextPropPath::ALL {
            p.copy(&other, &mut all);
        }
        assert_eq!(all, other);
    }
}
