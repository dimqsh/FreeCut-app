//! Text clip edits: content (TX-002), character style (TX-003, TX-004,
//! TX-008 to TX-016, TX-055), paragraph layout (TX-005, TX-009), card and
//! lens reset (TX-054) and **Change case** (TX-008).
//!
//! The style and layout edits follow the rules of the property edits in
//! [`super::props`]: one value on every listed text clip, other clips
//! skipped, refused when none is text or any is on a locked track, values
//! clamped into their ranges, unchanged clips recording nothing.

use std::ops::Range;
use std::sync::Arc;

use super::props::{try_update_clips, update_clips};
use crate::edit::tx::Tx;
use crate::edit::{EditError, EditRequest, OpKind, Plan};
use crate::ids::{ClipId, SeqId};
use crate::model::{
    BoxHeight, BoxShape, BubbleKind, CaseTransform, ClipContent, DeciDegrees, Document, Fill,
    FillCenter, FillExtent, FillKind, FontRef, GlowDirection, GradientStops, HAlign, LangTag,
    PerMille, PropGroup, PropPath, RefPx, Rgba8, RichText, SignedPerMille, StrokePosition, TailTip,
    TextCard, TextClipData, TextLayerKind, TextLayout, TextLens, TextPropPath, TextStyle,
    Transform, VAnchor, card_props, case,
};
use crate::motion::with_paced_duration;

/// One character-style value to put on text clips (TX-003, TX-004, TX-008
/// to TX-016, TX-055): one variant per property of the Style and Box &
/// bubble lenses (TX-054). Editing a layer's settings never switches it on
/// or off; [`TextStyleEdit::SetLayer`] does.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TextStyleEdit {
    Font(FontRef),
    /// Clamped to 4–1000 ref px.
    Size(RefPx),
    /// The whole fill: a solid colour or a gradient (TX-004, TX-014).
    Fill(Fill),
    /// Switches the fill's kind, keeping gradient stops ([`Fill::with_kind`]).
    FillKind(FillKind),
    /// The stops of a gradient fill; solid fills are left alone.
    FillStops(GradientStops),
    /// One stop's colour; fills without that stop are left alone.
    FillStopColor {
        index: u8,
        color: Rgba8,
    },
    /// A linear gradient's angle; other fills are left alone.
    FillAngle(DeciDegrees),
    /// A radial gradient's centre (0–1000 each); other fills are left alone.
    FillCenter(FillCenter),
    /// A radial gradient's spread, clamped to 100–2000; other fills are left
    /// alone.
    FillSpread(PerMille),
    /// A gradient's extent; solid fills are left alone.
    FillExtent(FillExtent),
    Bold(bool),
    Italic(bool),
    Underline(bool),
    Strike(bool),
    Case(CaseTransform),
    /// Clamped to −200..=1000 per-mille of the size (TX-009).
    LetterSpacing(SignedPerMille),
    /// Switches an optional layer (TX-054). Switching Background or Bubble
    /// on switches the other off in the same edit; switching any layer off
    /// keeps its settings. See [`TextStyle::with_layer`].
    SetLayer {
        layer: TextLayerKind,
        on: bool,
    },
    OutlineColor(Rgba8),
    /// Clamped to 0–300.
    OutlineWidth(PerMille),
    OutlineOpacity(PerMille),
    OutlinePosition(StrokePosition),
    GlowColor(Rgba8),
    GlowIntensity(PerMille),
    /// Clamped to 0–1000.
    GlowRadius(PerMille),
    /// Clamped to 0–500.
    GlowOffset(PerMille),
    GlowAngle(DeciDegrees),
    GlowDirection(GlowDirection),
    ShadowColor(Rgba8),
    ShadowOpacity(PerMille),
    /// Clamped to 0–500.
    ShadowBlur(PerMille),
    /// Clamped to 0–1000.
    ShadowDistance(PerMille),
    ShadowAngle(DeciDegrees),
    ShadowIncludeBox(bool),
    BackgroundColor(Rgba8),
    BackgroundOpacity(PerMille),
    BackgroundShape(BoxShape),
    BackgroundPadX(PerMille),
    BackgroundPadY(PerMille),
    BackgroundRoundness(PerMille),
    BackgroundHeight(BoxHeight),
    BackgroundBorder(bool),
    BackgroundBorderColor(Rgba8),
    /// Clamped to 0–200.
    BackgroundBorderWidth(PerMille),
    /// Also sets the tail switch to the kind's default
    /// ([`BubbleKind::tail_by_default`]); the tail's settings are kept.
    BubbleKind(BubbleKind),
    BubbleTail(bool),
    BubbleColor(Rgba8),
    BubbleOpacity(PerMille),
    /// Clamped to ±40,000 ref px per axis.
    BubbleTailTip(TailTip),
    /// Clamped to 100–600.
    BubbleTailBase(PerMille),
    /// Clamped to −1000..=1000.
    BubbleTailBend(SignedPerMille),
    BubblePadX(PerMille),
    BubblePadY(PerMille),
    BubbleRoundness(PerMille),
    BubbleBorder(bool),
    BubbleBorderColor(Rgba8),
    BubbleBorderWidth(PerMille),
    /// Clamped to 8–32.
    BubbleSpikes(u8),
    /// Clamped to 50–400.
    BubbleSpikeDepth(PerMille),
    BubbleJitter(PerMille),
    BubbleSeed(u32),
    /// Clamped to 100–400.
    BubbleLobe(PerMille),
    /// Clamped to 2–4.
    BubbleTrail(u8),
    /// The curve amount, clamped to −1000..=1000 (TX-016). The Curve layer's
    /// switch is [`TextStyleEdit::SetLayer`].
    Curve(SignedPerMille),
}

impl TextStyleEdit {
    pub(crate) fn applied_to(&self, style: &TextStyle) -> TextStyle {
        use TextStyleEdit as E;
        if let E::SetLayer { layer, on } = self {
            return style.with_layer(*layer, *on).clamped();
        }
        let mut s = style.clone();
        let stroke = &mut s.stroke.value;
        let glow = &mut s.glow.value;
        let shadow = &mut s.shadow.value;
        let bx = &mut s.background.value;
        let bu = &mut s.bubble.value;
        match self {
            E::SetLayer { .. } => {}
            E::Font(font) => s.font.clone_from(font),
            E::Size(size) => s.size = *size,
            E::Fill(fill) => s.fill.clone_from(fill),
            E::FillKind(kind) => s.fill = s.fill.with_kind(*kind),
            E::FillStops(stops) => match &mut s.fill {
                Fill::Solid(_) => {}
                Fill::LinearGradient(g) => g.stops.clone_from(stops),
                Fill::RadialGradient(g) => g.stops.clone_from(stops),
            },
            E::FillStopColor { index, color } => {
                let recoloured = s
                    .fill
                    .stops()
                    .and_then(|st| st.with_color(usize::from(*index), *color));
                match (&mut s.fill, recoloured) {
                    (Fill::LinearGradient(g), Some(st)) => g.stops = st,
                    (Fill::RadialGradient(g), Some(st)) => g.stops = st,
                    _ => {}
                }
            }
            E::FillAngle(angle) => {
                if let Fill::LinearGradient(g) = &mut s.fill {
                    g.angle = *angle;
                }
            }
            E::FillCenter(center) => {
                if let Fill::RadialGradient(g) = &mut s.fill {
                    g.center = *center;
                }
            }
            E::FillSpread(spread) => {
                if let Fill::RadialGradient(g) = &mut s.fill {
                    g.spread = *spread;
                }
            }
            E::FillExtent(extent) => match &mut s.fill {
                Fill::Solid(_) => {}
                Fill::LinearGradient(g) => g.extent = *extent,
                Fill::RadialGradient(g) => g.extent = *extent,
            },
            E::Bold(bold) => s.bold = *bold,
            E::Italic(italic) => s.italic = *italic,
            E::Underline(on) => s.decoration.underline = *on,
            E::Strike(on) => s.decoration.strike = *on,
            E::Case(case) => s.case = *case,
            E::LetterSpacing(v) => s.letter_spacing = *v,
            E::OutlineColor(c) => stroke.color = *c,
            E::OutlineWidth(v) => stroke.width = *v,
            E::OutlineOpacity(v) => stroke.opacity = *v,
            E::OutlinePosition(p) => stroke.position = *p,
            E::GlowColor(c) => glow.color = *c,
            E::GlowIntensity(v) => glow.intensity = *v,
            E::GlowRadius(v) => glow.radius = *v,
            E::GlowOffset(v) => glow.offset = *v,
            E::GlowAngle(a) => glow.angle = *a,
            E::GlowDirection(d) => glow.direction = *d,
            E::ShadowColor(c) => shadow.color = *c,
            E::ShadowOpacity(v) => shadow.opacity = *v,
            E::ShadowBlur(v) => shadow.blur = *v,
            E::ShadowDistance(v) => shadow.distance = *v,
            E::ShadowAngle(a) => shadow.angle = *a,
            E::ShadowIncludeBox(on) => shadow.include_box = *on,
            E::BackgroundColor(c) => bx.color = *c,
            E::BackgroundOpacity(v) => bx.opacity = *v,
            E::BackgroundShape(shape) => bx.shape = *shape,
            E::BackgroundPadX(v) => bx.pad_x = *v,
            E::BackgroundPadY(v) => bx.pad_y = *v,
            E::BackgroundRoundness(v) => bx.roundness = *v,
            E::BackgroundHeight(h) => bx.height = *h,
            E::BackgroundBorder(on) => bx.border.on = *on,
            E::BackgroundBorderColor(c) => bx.border.value.color = *c,
            E::BackgroundBorderWidth(v) => bx.border.value.width = *v,
            E::BubbleKind(kind) => {
                bu.spec.kind = *kind;
                bu.spec.tail.on = kind.tail_by_default();
            }
            E::BubbleTail(on) => bu.spec.tail.on = *on,
            E::BubbleColor(c) => bu.fill = *c,
            E::BubbleOpacity(v) => bu.opacity = *v,
            E::BubbleTailTip(tip) => bu.spec.tail.value.tip = *tip,
            E::BubbleTailBase(v) => bu.spec.tail.value.base = *v,
            E::BubbleTailBend(v) => bu.spec.tail.value.bend = *v,
            E::BubblePadX(v) => bu.pad_x = *v,
            E::BubblePadY(v) => bu.pad_y = *v,
            E::BubbleRoundness(v) => bu.roundness = *v,
            E::BubbleBorder(on) => bu.border.on = *on,
            E::BubbleBorderColor(c) => bu.border.value.color = *c,
            E::BubbleBorderWidth(v) => bu.border.value.width = *v,
            E::BubbleSpikes(n) => bu.spec.spikes = *n,
            E::BubbleSpikeDepth(v) => bu.spec.spike_depth = *v,
            E::BubbleJitter(v) => bu.spec.jitter = *v,
            E::BubbleSeed(seed) => bu.spec.seed = *seed,
            E::BubbleLobe(v) => bu.spec.lobe = *v,
            E::BubbleTrail(n) => bu.spec.trail = *n,
            E::Curve(v) => s.curve.value = *v,
        }
        s.clamped()
    }
}

/// One paragraph-layout value to put on text clips (TX-005, TX-009).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextLayoutEdit {
    /// Sets the alignment and switches justification off: the four
    /// alignment buttons are one choice (TX-005).
    Align(HAlign),
    /// Justified lines; the alignment is kept for when it is switched off.
    Justify(bool),
    /// `None` switches to auto width; a fixed width is clamped to
    /// 20–10,000 ref px.
    BoxWidth(Option<RefPx>),
    /// Clamped to 500–3000 per-mille.
    LineSpacing(PerMille),
    VAnchor(VAnchor),
}

impl TextLayoutEdit {
    pub(crate) fn applied_to(self, layout: TextLayout) -> TextLayout {
        match self {
            TextLayoutEdit::Align(align) => TextLayout {
                align,
                justify: false,
                ..layout
            },
            TextLayoutEdit::Justify(justify) => TextLayout { justify, ..layout },
            TextLayoutEdit::BoxWidth(width) => TextLayout {
                box_width: TextLayout::clamp_box_width(width),
                ..layout
            },
            TextLayoutEdit::LineSpacing(spacing) => TextLayout {
                line_spacing: TextLayout::clamp_line_spacing(spacing),
                ..layout
            },
            TextLayoutEdit::VAnchor(v_anchor) => TextLayout { v_anchor, ..layout },
        }
    }
}

/// What a [`EditRequest::ResetText`] resets (TX-054).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResetScope {
    /// One card's properties. The Text card has nothing to reset: content
    /// is never reset.
    Card(TextCard),
    /// Every card of a lens. The Content lens resets the paragraph only;
    /// the Box & bubble lens also resets its Placement card, the clip
    /// transform (TR-001, TR-015), to identity.
    Lens(TextLens),
}

impl ResetScope {
    fn cards(self) -> &'static [TextCard] {
        match self {
            ResetScope::Card(card) => match card {
                TextCard::Text => &[],
                TextCard::Paragraph => &[TextCard::Paragraph],
                TextCard::Type => &[TextCard::Type],
                TextCard::Fill => &[TextCard::Fill],
                TextCard::Spacing => &[TextCard::Spacing],
                TextCard::Outline => &[TextCard::Outline],
                TextCard::Glow => &[TextCard::Glow],
                TextCard::Shadow => &[TextCard::Shadow],
                TextCard::Background => &[TextCard::Background],
                TextCard::Bubble => &[TextCard::Bubble],
                TextCard::Curve => &[TextCard::Curve],
            },
            ResetScope::Lens(lens) => lens.cards(),
        }
    }
}

/// `data` with the properties of `scope`'s cards back at `defaults`',
/// except the content and the layers' on/off switches (TX-054). A bubble
/// that stays on with a reset colour follows TX-055's switch-on rule: a
/// colour the text fill contrasts with less than 3:1 becomes the dark
/// default, so the reset never hides the text.
pub(crate) fn reset_data(
    data: &TextClipData,
    defaults: &TextClipData,
    scope: ResetScope,
) -> TextClipData {
    let mut out = data.clone();
    for card in scope.cards() {
        for p in card_props(*card) {
            if *p != TextPropPath::Content && !p.is_layer_switch() {
                p.copy(defaults, &mut out);
            }
        }
    }
    if out.style.bubble.on && scope.cards().contains(&TextCard::Bubble) {
        let fill = out.style.readable_bubble_fill(out.style.bubble.value.fill);
        out.style.bubble.value.fill = fill;
    }
    out
}

/// Replaces the text content of one text clip (TX-002): one entry per edit
/// session. Setting the current content changes nothing. In and Out
/// animations with Pace get the length the new text needs, in the same
/// entry, so undo restores both (TX-022).
pub(crate) fn set_text_content(
    tx: Tx<'_>,
    seq: SeqId,
    clip: ClipId,
    content: &RichText,
) -> Result<Plan, EditError> {
    let rate = tx.rate();
    try_update_clips(
        tx,
        seq,
        &[clip],
        OpKind::SetTextContent,
        EditError::NoTextClip,
        |clip| {
            let ClipContent::Text(data) = &clip.content else {
                return Ok(None);
            };
            let mut out = clip.clone();
            out.content = ClipContent::Text(Arc::new(TextClipData {
                content: content.clone(),
                ..TextClipData::clone(data)
            }));
            let text = content.as_str();
            for slot in [&mut out.animations.entrance, &mut out.animations.exit] {
                if let Some(spec) = slot.as_mut()
                    && spec.params.pace.is_some()
                {
                    *spec = Arc::new(with_paced_duration(spec, text, rate)?);
                }
            }
            Ok(Some(out))
        },
    )
}

pub(crate) fn set_text_style(
    tx: Tx<'_>,
    seq: SeqId,
    clips: &[ClipId],
    edit: &TextStyleEdit,
) -> Result<Plan, EditError> {
    update_text(tx, seq, clips, OpKind::SetTextStyle, |data| TextClipData {
        style: edit.applied_to(&data.style),
        ..data.clone()
    })
}

pub(crate) fn set_text_layout(
    tx: Tx<'_>,
    seq: SeqId,
    clips: &[ClipId],
    edit: TextLayoutEdit,
) -> Result<Plan, EditError> {
    update_text(tx, seq, clips, OpKind::SetTextLayout, |data| TextClipData {
        layout: edit.applied_to(data.layout),
        ..data.clone()
    })
}

/// Card and lens reset (TX-054) to the factory default text style, with
/// the scope's keyframes removed (the Box & bubble lens also resets the
/// Placement card's transform and its keys). One entry for every listed
/// text clip.
pub(crate) fn reset_text(
    tx: Tx<'_>,
    seq: SeqId,
    clips: &[ClipId],
    scope: ResetScope,
) -> Result<Plan, EditError> {
    let defaults = TextClipData::default();
    let placement = scope == ResetScope::Lens(TextLens::BoxAndBubble);
    update_clips(
        tx,
        seq,
        clips,
        OpKind::ResetText,
        EditError::NoTextClip,
        |clip| {
            let ClipContent::Text(data) = &clip.content else {
                return None;
            };
            let mut out = clip.clone();
            out.content = ClipContent::Text(Arc::new(reset_data(data, &defaults, scope)));
            if placement {
                out.transform = Transform::IDENTITY;
            }
            let cards = scope.cards();
            out.keys = clip.keys.without(|p: PropPath| match p.group() {
                Some(PropGroup::Text(card)) => cards.contains(&card),
                Some(PropGroup::Transform) => placement,
                Some(PropGroup::Color(_)) | None => false,
            });
            Some(out)
        },
    )
}

/// The **Change case** command (TX-008): a [`EditRequest::SetTextContent`]
/// that rewrites the byte range `selection` of `clip`'s stored content in
/// `case` (see [`case::change_range`]). Until per-range styling (TX-015)
/// the whole new content is sent, built from the content field's selection.
pub fn change_case(
    doc: &Document,
    seq: SeqId,
    clip: ClipId,
    selection: Range<usize>,
    case: CaseTransform,
    lang: Option<&LangTag>,
) -> Result<EditRequest, EditError> {
    let loc = doc.find_clip(clip).ok_or(EditError::StaleClip(clip))?;
    if loc.seq != seq {
        return Err(EditError::StaleClip(clip));
    }
    let data = doc
        .clip(clip)
        .and_then(|c| c.content.text())
        .ok_or(EditError::NoTextClip)?;
    let text = case::change_range(data.content.as_str(), selection, case, lang);
    Ok(EditRequest::SetTextContent {
        seq,
        clip,
        content: RichText::new(text),
    })
}

/// Applies `f` to the text data of every listed text clip.
fn update_text(
    tx: Tx<'_>,
    seq: SeqId,
    clips: &[ClipId],
    op: OpKind,
    f: impl Fn(&TextClipData) -> TextClipData,
) -> Result<Plan, EditError> {
    update_clips(tx, seq, clips, op, EditError::NoTextClip, |clip| {
        let ClipContent::Text(data) = &clip.content else {
            return None;
        };
        let mut out = clip.clone();
        out.content = ClipContent::Text(Arc::new(f(data)));
        Some(out)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BubbleSpec, Layer, Stroke, TextBox, TextBubble};

    #[test]
    fn text_edits_clamp() {
        let style = TextStyleEdit::Size(RefPx::from_px(2)).applied_to(&TextStyle::default());
        assert_eq!(style.size, TextStyle::MIN_SIZE);
        let layout = TextLayoutEdit::BoxWidth(Some(RefPx::from_px(20_000)))
            .applied_to(TextLayout::default());
        assert_eq!(layout.box_width, Some(TextLayout::MAX_BOX_WIDTH));
        let layout = TextLayoutEdit::LineSpacing(PerMille::new(100)).applied_to(layout);
        assert_eq!(layout.line_spacing, TextLayout::MIN_LINE_SPACING);
        let s = TextStyleEdit::OutlineWidth(PerMille::new(5000)).applied_to(&style);
        assert_eq!(s.stroke.value.width, Stroke::MAX_WIDTH);
        let s = TextStyleEdit::BubbleSpikes(0).applied_to(&s);
        assert_eq!(s.bubble.value.spec.spikes, BubbleSpec::MIN_SPIKES);
        let s = TextStyleEdit::LetterSpacing(SignedPerMille::new(i16::MIN)).applied_to(&s);
        assert_eq!(s.letter_spacing, TextStyle::MIN_LETTER_SPACING);
    }

    #[test]
    fn align_and_justify_are_one_choice_that_remembers_the_alignment() {
        let l = TextLayoutEdit::Align(HAlign::Right).applied_to(TextLayout::default());
        let j = TextLayoutEdit::Justify(true).applied_to(l);
        assert!(j.justify);
        assert_eq!(j.align, HAlign::Right);
        assert_eq!(TextLayoutEdit::Justify(false).applied_to(j), l);
        let a = TextLayoutEdit::Align(HAlign::Left).applied_to(j);
        assert!(!a.justify);
        assert_eq!(a.align, HAlign::Left);
    }

    #[test]
    fn fill_part_edits_skip_fills_without_that_part() {
        let solid = TextStyle::default();
        for edit in [
            TextStyleEdit::FillAngle(DeciDegrees::ZERO),
            TextStyleEdit::FillCenter(FillCenter::MIDDLE),
            TextStyleEdit::FillSpread(PerMille::new(300)),
            TextStyleEdit::FillExtent(FillExtent::Line),
            TextStyleEdit::FillStops(GradientStops::two(Rgba8::BLACK, Rgba8::WHITE)),
            TextStyleEdit::FillStopColor {
                index: 0,
                color: Rgba8::BLACK,
            },
        ] {
            assert_eq!(edit.applied_to(&solid), solid, "{edit:?}");
        }
        let linear = TextStyleEdit::FillKind(FillKind::LinearGradient).applied_to(&solid);
        let s = TextStyleEdit::FillAngle(DeciDegrees::new(450)).applied_to(&linear);
        let Fill::LinearGradient(g) = &s.fill else {
            panic!("not linear");
        };
        assert_eq!(g.angle.tenths(), 450);
        let s = TextStyleEdit::FillStopColor {
            index: 1,
            color: Rgba8::new(1, 2, 3, 4),
        }
        .applied_to(&s);
        assert_eq!(
            s.fill.stops().unwrap().as_slice()[1].color,
            Rgba8::new(1, 2, 3, 4)
        );
        let same = TextStyleEdit::FillStopColor {
            index: 7,
            color: Rgba8::BLACK,
        }
        .applied_to(&s);
        assert_eq!(same, s);
        let radial = TextStyleEdit::FillKind(FillKind::RadialGradient).applied_to(&s);
        let s = TextStyleEdit::FillSpread(PerMille::new(9)).applied_to(&radial);
        let Fill::RadialGradient(g) = &s.fill else {
            panic!("not radial");
        };
        assert_eq!(g.spread.get(), 100);
    }

    #[test]
    fn bubble_kind_sets_the_tail_switch_and_keeps_the_tail() {
        let mut s = TextStyle::default();
        s.bubble.value.spec.tail.value.bend = SignedPerMille::new(-300);
        let tag = TextStyleEdit::BubbleKind(BubbleKind::Tag).applied_to(&s);
        assert!(!tag.bubble.value.spec.tail.on);
        assert_eq!(tag.bubble.value.spec.tail.value.bend.get(), -300);
        let oval = TextStyleEdit::BubbleKind(BubbleKind::Oval).applied_to(&tag);
        assert!(oval.bubble.value.spec.tail.on);
    }

    #[test]
    fn reset_keeps_content_and_layer_switches() {
        let mut d = TextClipData::with_content("keep me");
        d.style.stroke = Layer::on(Stroke {
            width: PerMille::new(200),
            ..Stroke::default()
        });
        d.style.background = Layer::on(TextBox {
            opacity: PerMille::new(100),
            ..TextBox::default()
        });
        d.style.bubble = Layer::off(TextBubble {
            fill: Rgba8::BLACK,
            ..TextBubble::default()
        });
        d.layout.justify = true;
        d.style.size = RefPx::from_px(200);
        let defaults = TextClipData::default();
        let r = reset_data(&d, &defaults, ResetScope::Card(TextCard::Outline));
        assert!(r.style.stroke.on);
        assert_eq!(r.style.stroke.value, Stroke::default());
        assert_eq!(r.style.size, RefPx::from_px(200));
        let r = reset_data(&d, &defaults, ResetScope::Lens(TextLens::BoxAndBubble));
        assert!(r.style.background.on && !r.style.bubble.on);
        assert_eq!(r.style.background.value, TextBox::default());
        assert_eq!(r.style.bubble.value, TextBubble::default());
        let r = reset_data(&d, &defaults, ResetScope::Lens(TextLens::Content));
        assert_eq!(r.content.as_str(), "keep me");
        assert!(!r.layout.justify);
        assert_eq!(
            reset_data(&d, &defaults, ResetScope::Card(TextCard::Text)),
            d
        );
    }

    #[test]
    fn reset_of_a_bubble_that_stays_on_keeps_the_text_readable() {
        // White text in a bubble switched on: the bubble took the dark
        // default (TX-055). Resetting the card or the lens must not bring
        // back the white factory fill behind the white text.
        let mut d = TextClipData::with_content("hi");
        d.style = d.style.with_layer(TextLayerKind::Bubble, true);
        d.style.bubble.value.fill = Rgba8::new(200, 0, 0, 255);
        let defaults = TextClipData::default();
        for scope in [
            ResetScope::Card(TextCard::Bubble),
            ResetScope::Lens(TextLens::BoxAndBubble),
        ] {
            let r = reset_data(&d, &defaults, scope);
            assert!(r.style.bubble.on, "{scope:?}");
            assert_eq!(r.style.bubble.value.fill, Rgba8::DARK, "{scope:?}");
        }
        // Dark text keeps the factory white bubble.
        d.style.fill = Fill::Solid(Rgba8::BLACK);
        let r = reset_data(&d, &defaults, ResetScope::Card(TextCard::Bubble));
        assert_eq!(r.style.bubble.value.fill, Rgba8::WHITE);
        // A bubble that is off keeps the factory fill: switching it on
        // applies the rule then.
        d.style.fill = Fill::Solid(Rgba8::WHITE);
        d.style.bubble.on = false;
        let r = reset_data(&d, &defaults, ResetScope::Card(TextCard::Bubble));
        assert_eq!(r.style.bubble.value.fill, Rgba8::WHITE);
    }
}
