//! Text clip payload (TX-001 to TX-016, TX-054, TX-055).
//!
//! Style scalars are fixed-point integers (TX conventions): lengths in 1/100
//! ref px, ratios in per-mille (signed ones in [`SignedPerMille`]), angles in
//! tenths of a degree ([`DeciDegrees`]). A ref px is a pixel on a reference
//! canvas whose short edge is 1080 px; the renderer multiplies by
//! `canvas_short_edge / 1080`. Colors are sRGB with straight alpha.

mod bubble;
pub mod case;
mod contrast;
mod props;
mod style;
mod units;

use crate::time::{FrameRate, TICKS_PER_SECOND, Tick, TimeError};

use super::transform::PerMille;

pub use bubble::{BubbleKind, BubbleSpec, Side, Tail, TailTip, TextBubble};
pub use props::{TextCard, TextLens, TextPropPath, card_props};
pub use style::{
    Border, BoxHeight, BoxShape, CaseTransform, Decoration, Fill, FillCenter, FillExtent, FillKind,
    Glow, GlowDirection, GradientError, GradientStop, GradientStops, Layer, LinearGradient,
    RadialGradient, Shadow, Stroke, StrokePosition, TextBox,
};
pub use units::{DeciDegrees, SignedPerMille};

/// The length of a new text clip before rounding to the frame grid: 3 s
/// (TX-001, FreeCut default).
pub const DEFAULT_TEXT_DURATION: Tick = Tick::from_raw(3 * TICKS_PER_SECOND);

/// The default text clip length on `rate`'s frame grid (TX-001).
pub fn default_text_duration(rate: FrameRate) -> Result<Tick, TimeError> {
    let t = rate.quantize(DEFAULT_TEXT_DURATION)?;
    // Even the slowest rate has frames shorter than 3 s; one frame minimum
    // keeps the result a valid clip length regardless.
    Ok(t.max(rate.frame_ticks()))
}

/// The text of a clip, stored exactly as typed: UTF-8, no normalization
/// (TX-002). Plain text for now; per-range styles (TX-015) will be added
/// here without changing how clips hold it.
///
/// The model accepts any length. The 10,000-character limit of TX-002 is a
/// soft limit enforced by the editing UI.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct RichText(String);

impl RichText {
    pub fn new(text: impl Into<String>) -> RichText {
        RichText(text.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Length in bytes.
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl From<&str> for RichText {
    fn from(text: &str) -> RichText {
        RichText::new(text)
    }
}

impl From<String> for RichText {
    fn from(text: String) -> RichText {
        RichText(text)
    }
}

/// An sRGB color with straight (not premultiplied) alpha.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgba8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba8 {
    pub const WHITE: Rgba8 = Rgba8::new(255, 255, 255, 255);
    pub const BLACK: Rgba8 = Rgba8::new(0, 0, 0, 255);
    /// `#1E1E24`, the dark default of bubbles and their borders (TX-055).
    pub const DARK: Rgba8 = Rgba8::new(0x1E, 0x1E, 0x24, 255);

    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Rgba8 {
        Rgba8 { r, g, b, a }
    }
}

/// A length in hundredths of a ref px (TX conventions).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RefPx(i32);

impl RefPx {
    pub const fn from_hundredths(hundredths: i32) -> RefPx {
        RefPx(hundredths)
    }

    /// Whole ref px.
    pub const fn from_px(px: i32) -> RefPx {
        RefPx(px.saturating_mul(100))
    }

    pub const fn hundredths(self) -> i32 {
        self.0
    }

    /// The length in ref px, for the render and UI edge.
    pub fn as_f64(self) -> f64 {
        f64::from(self.0) / 100.0
    }

    fn clamp(self, min: RefPx, max: RefPx) -> RefPx {
        RefPx(self.0.clamp(min.0, max.0))
    }
}

/// A font, by family name (TX-003). Faces (weight, stretch) and the
/// PostScript-name hint come with the full font picker; bold and italic are
/// style toggles for now.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FontRef {
    pub family: String,
}

impl FontRef {
    /// The bundled sans family (Noto Sans, OFL-1.1).
    pub const DEFAULT_FAMILY: &'static str = "Noto Sans";

    pub fn new(family: impl Into<String>) -> FontRef {
        FontRef {
            family: family.into(),
        }
    }
}

impl Default for FontRef {
    fn default() -> FontRef {
        FontRef::new(FontRef::DEFAULT_FAMILY)
    }
}

/// A BCP 47 language tag (TX-006), such as `tr-TR`. Stored as given; only
/// the primary language subtag is interpreted here (case mapping, TX-008).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LangTag(String);

impl LangTag {
    pub fn new(tag: impl Into<String>) -> LangTag {
        LangTag(tag.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The primary language subtag: `tr` of `tr-TR`.
    pub fn primary(&self) -> &str {
        self.0.split(['-', '_']).next().unwrap_or_default()
    }

    /// Turkish and Azerbaijani case dotted and dotless i their own way.
    pub fn uses_turkic_casing(&self) -> bool {
        let p = self.primary();
        p.eq_ignore_ascii_case("tr") || p.eq_ignore_ascii_case("az")
    }
}

/// Which optional layer of a text style (TX-054).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextLayerKind {
    Outline,
    Shadow,
    Glow,
    Background,
    Bubble,
    Curve,
}

/// Character style of a whole clip (TX-003, TX-004, TX-008 to TX-016,
/// TX-055).
///
/// Invariant (TX-054): `background` and `bubble` are never both on.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextStyle {
    pub font: FontRef,
    /// 4–1000 ref px.
    pub size: RefPx,
    pub fill: Fill,
    pub bold: bool,
    pub italic: bool,
    pub decoration: Decoration,
    pub case: CaseTransform,
    /// Added after each grapheme cluster: −200..=1000 per-mille of the size
    /// (TX-009).
    pub letter_spacing: SignedPerMille,
    pub stroke: Layer<Stroke>,
    pub shadow: Layer<Shadow>,
    pub glow: Layer<Glow>,
    pub background: Layer<TextBox>,
    pub bubble: Layer<TextBubble>,
    /// −1000..=1000, the UI's −100 to +100 (TX-016).
    pub curve: Layer<SignedPerMille>,
}

impl TextStyle {
    pub const MIN_SIZE: RefPx = RefPx::from_px(4);
    pub const MAX_SIZE: RefPx = RefPx::from_px(1000);
    pub const DEFAULT_SIZE: RefPx = RefPx::from_px(64);
    pub const MIN_LETTER_SPACING: SignedPerMille = SignedPerMille::new(-200);
    pub const MAX_LETTER_SPACING: SignedPerMille = SignedPerMille::new(1000);
    pub const MAX_CURVE: SignedPerMille = SignedPerMille::new(1000);

    /// `size` clamped to the allowed range (TX-004).
    pub fn clamp_size(size: RefPx) -> RefPx {
        size.clamp(Self::MIN_SIZE, Self::MAX_SIZE)
    }

    /// Every value inside its range, and not both box and bubble on.
    pub fn is_valid(&self) -> bool {
        self.clamped() == *self
    }

    /// The same style with every value clamped into its range. With both
    /// Background and Bubble on (a hand-edited or newer file), the bubble
    /// wins and Background is switched off (TX-054).
    pub fn clamped(&self) -> TextStyle {
        let c = |v: SignedPerMille, max: SignedPerMille| v.clamp(-max.get(), max.get());
        let mut out = TextStyle {
            font: self.font.clone(),
            size: Self::clamp_size(self.size),
            fill: self.fill.clamped(),
            bold: self.bold,
            italic: self.italic,
            decoration: self.decoration,
            case: self.case,
            letter_spacing: self.letter_spacing.clamp(
                Self::MIN_LETTER_SPACING.get(),
                Self::MAX_LETTER_SPACING.get(),
            ),
            stroke: self.stroke.map(Stroke::clamped),
            shadow: self.shadow.map(Shadow::clamped),
            glow: self.glow.map(Glow::clamped),
            background: self.background.map(TextBox::clamped),
            bubble: self.bubble.map(TextBubble::clamped),
            curve: self.curve.map(|v| c(v, Self::MAX_CURVE)),
        };
        if out.background.on && out.bubble.on {
            out.background.on = false;
        }
        out
    }

    /// Whether the text is laid out on an arc: the Curve layer is on with a
    /// non-zero amount (TX-054). Curved text pauses the fixed width
    /// (TX-005), the background box and the bubble (TX-013, TX-055).
    pub fn is_curved(&self) -> bool {
        self.curve
            .active()
            .is_some_and(|v| *v != SignedPerMille::ZERO)
    }

    /// The style with `layer` switched `on` (TX-054). Switching Background
    /// or Bubble on switches the other off. A bubble switched on over a fill
    /// it would contrast with less than 3:1 takes the dark default fill
    /// (TX-055); the text colour never changes. Switching off keeps the
    /// layer's settings.
    pub fn with_layer(&self, layer: TextLayerKind, on: bool) -> TextStyle {
        let mut out = self.clone();
        match layer {
            TextLayerKind::Outline => out.stroke.on = on,
            TextLayerKind::Shadow => out.shadow.on = on,
            TextLayerKind::Glow => out.glow.on = on,
            TextLayerKind::Curve => out.curve.on = on,
            TextLayerKind::Background => {
                out.background.on = on;
                if on {
                    out.bubble.on = false;
                }
            }
            TextLayerKind::Bubble => {
                if on && !self.bubble.on {
                    out.background.on = false;
                    out.bubble.value.fill = self.readable_bubble_fill(out.bubble.value.fill);
                }
                out.bubble.on = on;
            }
        }
        out
    }

    /// `bubble`, or the dark default `#1E1E24` when the text fill would
    /// contrast with it less than 3:1 (TX-055). With a gradient every stop
    /// must reach 3:1.
    pub(crate) fn readable_bubble_fill(&self, bubble: Rgba8) -> Rgba8 {
        let readable = self
            .fill
            .colors()
            .into_iter()
            .all(|c| contrast::contrasts(c, bubble, 30));
        if readable { bubble } else { Rgba8::DARK }
    }

    /// The on/off state of `layer`.
    pub fn layer_on(&self, layer: TextLayerKind) -> bool {
        match layer {
            TextLayerKind::Outline => self.stroke.on,
            TextLayerKind::Shadow => self.shadow.on,
            TextLayerKind::Glow => self.glow.on,
            TextLayerKind::Background => self.background.on,
            TextLayerKind::Bubble => self.bubble.on,
            TextLayerKind::Curve => self.curve.on,
        }
    }
}

impl Default for TextStyle {
    /// The factory default (TX-001): Noto Sans Bold, 64 ref px, white, no
    /// decoration, as typed, no letter spacing, every optional layer off
    /// with its own defaults (TX-010 to TX-016, TX-055).
    fn default() -> TextStyle {
        TextStyle {
            font: FontRef::default(),
            size: Self::DEFAULT_SIZE,
            fill: Fill::Solid(Rgba8::WHITE),
            bold: true,
            italic: false,
            decoration: Decoration::default(),
            case: CaseTransform::AsTyped,
            letter_spacing: SignedPerMille::ZERO,
            stroke: Layer::off(Stroke::default()),
            shadow: Layer::off(Shadow::default()),
            glow: Layer::off(Glow::default()),
            background: Layer::off(TextBox::default()),
            bubble: Layer::off(TextBubble::default()),
            curve: Layer::off(SignedPerMille::ZERO),
        }
    }
}

/// Horizontal alignment of the lines in the box (TX-005). Visual: left
/// means left, also in right-to-left text. Justification is
/// [`TextLayout::justify`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum HAlign {
    Left,
    #[default]
    Center,
    Right,
}

/// Which edge of the text box stays put as lines are added or removed
/// (TX-005): the layer origin's vertical position in the box.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum VAnchor {
    Top,
    #[default]
    Middle,
    Bottom,
}

/// Paragraph layout of a whole clip (TX-005, TX-009).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextLayout {
    /// The alignment, kept while `justify` is on and used again when it is
    /// switched off.
    pub align: HAlign,
    /// Justified lines (TX-005): every line but the last of each paragraph
    /// spans the box; the last line is start-aligned. Wins over `align`.
    pub justify: bool,
    /// `None`: auto width, lines break only at manual breaks. `Some`: fixed
    /// width, 20–10,000 ref px, text wraps.
    pub box_width: Option<RefPx>,
    /// Factor of the font's natural line height, 500–3000 per-mille.
    pub line_spacing: PerMille,
    pub v_anchor: VAnchor,
}

impl TextLayout {
    pub const MIN_BOX_WIDTH: RefPx = RefPx::from_px(20);
    pub const MAX_BOX_WIDTH: RefPx = RefPx::from_px(10_000);
    pub const MIN_LINE_SPACING: PerMille = PerMille::new(500);
    pub const MAX_LINE_SPACING: PerMille = PerMille::new(3000);

    pub fn clamp_box_width(width: Option<RefPx>) -> Option<RefPx> {
        width.map(|w| w.clamp(Self::MIN_BOX_WIDTH, Self::MAX_BOX_WIDTH))
    }

    pub fn clamp_line_spacing(spacing: PerMille) -> PerMille {
        PerMille::new(
            spacing
                .get()
                .clamp(Self::MIN_LINE_SPACING.get(), Self::MAX_LINE_SPACING.get()),
        )
    }

    pub fn is_valid(&self) -> bool {
        Self::clamp_box_width(self.box_width) == self.box_width
            && Self::clamp_line_spacing(self.line_spacing) == self.line_spacing
    }
}

impl Default for TextLayout {
    /// Centered, not justified, auto width, 100 % line spacing, anchored
    /// in the middle (TX-005, TX-009).
    fn default() -> TextLayout {
        TextLayout {
            align: HAlign::Center,
            justify: false,
            box_width: None,
            line_spacing: PerMille::ONE,
            v_anchor: VAnchor::Middle,
        }
    }
}

/// Where a text clip came from (TX-001). Templates (TX-024) and converted
/// captions (TX-035) add variants.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TextOrigin {
    #[default]
    Manual,
}

/// Everything a text clip shows (TX-001). Its placement on the canvas is
/// the clip's [`Transform`](super::Transform), like any visual clip.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextClipData {
    pub content: RichText,
    pub style: TextStyle,
    pub layout: TextLayout,
    pub origin: TextOrigin,
}

impl TextClipData {
    /// The placeholder of a new text clip (TX-001, FreeCut default).
    pub const PLACEHOLDER: &'static str = "Your text";

    /// `content` with the factory style and layout.
    pub fn with_content(content: impl Into<RichText>) -> TextClipData {
        TextClipData {
            content: content.into(),
            style: TextStyle::default(),
            layout: TextLayout::default(),
            origin: TextOrigin::Manual,
        }
    }

    /// Style and layout inside their ranges, box and bubble not both on.
    pub fn is_valid(&self) -> bool {
        self.style.is_valid() && self.layout.is_valid()
    }

    /// The same data with every scalar clamped into its range and the
    /// box/bubble exclusivity restored, for loaders of older or hand-edited
    /// files.
    pub fn clamped(&self) -> TextClipData {
        let mut out = self.clone();
        out.style = out.style.clamped();
        out.layout.box_width = TextLayout::clamp_box_width(out.layout.box_width);
        out.layout.line_spacing = TextLayout::clamp_line_spacing(out.layout.line_spacing);
        out
    }
}

impl Default for TextClipData {
    /// "Your text" in the factory style (TX-001).
    fn default() -> TextClipData {
        TextClipData::with_content(TextClipData::PLACEHOLDER)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_defaults_match_tx_001() {
        let d = TextClipData::default();
        assert_eq!(d.content.as_str(), "Your text");
        assert_eq!(d.style.font.family, "Noto Sans");
        assert_eq!(d.style.size.hundredths(), 6400);
        assert_eq!(d.style.fill, Fill::Solid(Rgba8::new(255, 255, 255, 255)));
        assert!(d.style.bold && !d.style.italic);
        assert_eq!(d.style.decoration, Decoration::default());
        assert_eq!(d.style.case, CaseTransform::AsTyped);
        assert_eq!(d.style.letter_spacing, SignedPerMille::ZERO);
        // No stroke, shadow, glow or box (TX-001).
        assert!(!d.style.stroke.on && !d.style.shadow.on && !d.style.glow.on);
        assert!(!d.style.background.on && !d.style.bubble.on && !d.style.curve.on);
        assert!(!d.style.is_curved());
        assert_eq!(d.layout.align, HAlign::Center);
        assert!(!d.layout.justify);
        assert_eq!(d.layout.box_width, None);
        assert_eq!(d.layout.line_spacing.get(), 1000);
        assert_eq!(d.layout.v_anchor, VAnchor::Middle);
        assert_eq!(d.origin, TextOrigin::Manual);
        assert!(d.is_valid());
    }

    #[test]
    fn default_duration_is_three_seconds_on_the_grid() {
        let r25 = FrameRate::new(25, 1).unwrap();
        assert_eq!(default_text_duration(r25).unwrap(), r25.frames(75).unwrap());
        // 3 s at 23.976 is 71.93 frames: rounds to 72.
        let ntsc = FrameRate::new(24_000, 1001).unwrap();
        assert_eq!(
            default_text_duration(ntsc).unwrap(),
            ntsc.frames(72).unwrap()
        );
    }

    #[test]
    fn clamping_puts_scalars_in_range() {
        let mut d = TextClipData::default();
        d.style.size = RefPx::from_px(5000);
        d.layout.box_width = Some(RefPx::from_px(1));
        d.layout.line_spacing = PerMille::new(9000);
        assert!(!d.is_valid());
        let c = d.clamped();
        assert!(c.is_valid());
        assert_eq!(c.style.size, TextStyle::MAX_SIZE);
        assert_eq!(c.layout.box_width, Some(TextLayout::MIN_BOX_WIDTH));
        assert_eq!(c.layout.line_spacing, TextLayout::MAX_LINE_SPACING);
    }

    #[test]
    fn layer_defaults_match_the_spec() {
        let s = TextStyle::default();
        assert_eq!(s.stroke.value.width.get(), 60);
        assert_eq!(s.stroke.value.color, Rgba8::BLACK);
        assert_eq!(s.shadow.value.opacity.get(), 500);
        assert_eq!(s.shadow.value.blur.get(), 100);
        assert_eq!(s.shadow.value.distance.get(), 80);
        assert_eq!(s.shadow.value.angle.tenths(), 450);
        assert_eq!(s.glow.value.intensity.get(), 600);
        assert_eq!(s.glow.value.radius.get(), 250);
        assert_eq!(s.glow.value.direction, GlowDirection::Outward);
        let b = s.background.value;
        assert_eq!(
            (b.opacity.get(), b.pad_x.get(), b.pad_y.get()),
            (700, 250, 150)
        );
        assert_eq!((b.roundness.get(), b.shape), (200, BoxShape::Block));
        assert!(!b.border.on);
        assert_eq!(b.border.value.width.get(), 30);
        let u = s.bubble.value;
        assert_eq!(u.spec.kind, BubbleKind::Rounded);
        assert!(u.spec.tail.on);
        assert_eq!(u.spec.tail.value.base.get(), 240);
        assert_eq!(u.spec.tail.value.bend.get(), 200);
        assert_eq!(
            (u.pad_x.get(), u.pad_y.get(), u.roundness.get()),
            (450, 300, 400)
        );
        assert_eq!((u.spec.spikes, u.spec.spike_depth.get()), (14, 180));
        assert_eq!((u.spec.lobe.get(), u.spec.trail), (220, 3));
        assert!(u.border.on);
        assert_eq!(u.border.value.width.get(), 40);
        assert!(s.is_valid());
    }

    #[test]
    fn background_and_bubble_are_exclusive() {
        let s = TextStyle::default().with_layer(TextLayerKind::Background, true);
        assert!(s.background.on && !s.bubble.on);
        let s = s.with_layer(TextLayerKind::Bubble, true);
        assert!(!s.background.on && s.bubble.on);
        let s = s.with_layer(TextLayerKind::Background, true);
        assert!(s.background.on && !s.bubble.on);
        // Off keeps the other layer as it is.
        let s = s.with_layer(TextLayerKind::Background, false);
        assert!(!s.background.on && !s.bubble.on);
        // Both on is invalid; clamping lets the bubble win.
        let mut both = TextStyle::default();
        both.background.on = true;
        both.bubble.on = true;
        assert!(!both.is_valid());
        let fixed = both.clamped();
        assert!(fixed.bubble.on && !fixed.background.on);
    }

    #[test]
    fn a_bubble_under_white_text_turns_dark() {
        // White text on the white default bubble: under 3:1.
        let s = TextStyle::default().with_layer(TextLayerKind::Bubble, true);
        assert_eq!(s.bubble.value.fill, Rgba8::DARK);
        assert_eq!(
            s.fill,
            Fill::Solid(Rgba8::WHITE),
            "text colour never changes"
        );
        // Dark text keeps the white bubble.
        let dark = TextStyle {
            fill: Fill::Solid(Rgba8::BLACK),
            ..TextStyle::default()
        };
        let s = dark.with_layer(TextLayerKind::Bubble, true);
        assert_eq!(s.bubble.value.fill, Rgba8::WHITE);
        // A gradient with one light stop counts as unreadable.
        let grad = TextStyle {
            fill: Fill::LinearGradient(LinearGradient {
                angle: DeciDegrees::DOWN,
                stops: GradientStops::two(Rgba8::BLACK, Rgba8::WHITE),
                extent: FillExtent::Block,
            }),
            ..TextStyle::default()
        };
        let s = grad.with_layer(TextLayerKind::Bubble, true);
        assert_eq!(s.bubble.value.fill, Rgba8::DARK);
        // Switching on again when already on changes nothing.
        let again = s.with_layer(TextLayerKind::Bubble, true);
        assert_eq!(again, s);
    }

    #[test]
    fn curved_means_on_and_non_zero() {
        let mut s = TextStyle::default();
        s.curve.value = SignedPerMille::new(300);
        assert!(!s.is_curved());
        s.curve.on = true;
        assert!(s.is_curved());
        s.curve.value = SignedPerMille::ZERO;
        assert!(!s.is_curved());
    }

    #[test]
    fn style_values_clamp_into_their_ranges() {
        let mut s = TextStyle {
            letter_spacing: SignedPerMille::new(-500),
            ..TextStyle::default()
        };
        s.curve.value = SignedPerMille::new(i16::MAX);
        s.stroke.value.width = PerMille::new(999);
        s.shadow.value.blur = PerMille::new(999);
        s.glow.value.offset = PerMille::new(999);
        s.background.value.border.value.width = PerMille::new(999);
        s.bubble.value.spec.spikes = 200;
        s.bubble.value.spec.trail = 0;
        s.bubble.value.spec.tail.value.bend = SignedPerMille::new(-5000);
        s.bubble.value.spec.tail.value.tip.x = RefPx::from_px(i32::MAX / 100);
        assert!(!s.is_valid());
        let c = s.clamped();
        assert!(c.is_valid());
        assert_eq!(c.letter_spacing.get(), -200);
        assert_eq!(c.curve.value.get(), 1000);
        assert_eq!(c.stroke.value.width, Stroke::MAX_WIDTH);
        assert_eq!(c.shadow.value.blur, Shadow::MAX_BLUR);
        assert_eq!(c.glow.value.offset, Glow::MAX_OFFSET);
        assert_eq!(c.background.value.border.value.width, Border::MAX_WIDTH);
        assert_eq!(c.bubble.value.spec.spikes, BubbleSpec::MAX_SPIKES);
        assert_eq!(c.bubble.value.spec.trail, BubbleSpec::MIN_TRAIL);
        assert_eq!(c.bubble.value.spec.tail.value.bend.get(), -1000);
        assert_eq!(c.bubble.value.spec.tail.value.tip.x, TailTip::LIMIT);
    }

    #[test]
    fn language_tags() {
        assert!(LangTag::new("tr").uses_turkic_casing());
        assert!(LangTag::new("AZ-Latn").uses_turkic_casing());
        assert!(!LangTag::new("en-TR").uses_turkic_casing());
        assert_eq!(LangTag::new("pt_BR").primary(), "pt");
        assert_eq!(LangTag::new("").primary(), "");
    }
}
