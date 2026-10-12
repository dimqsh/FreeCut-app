//! The render-owned text style (TX-008 … TX-014, TX-016, TX-055).
//!
//! These mirror the timeline's text model, decoupled from it: the adapter
//! that builds a [`TextSpec`](super::TextSpec) turns the model's
//! fixed-point values into these floats (the render edge), resolves
//! `Layer { on, value }` into `Option` (a layer that is off is `None`) and
//! applies the case transform to the text.
//!
//! **Units.** Lengths that the spec defines relative to the font size are
//! *fractions of the font size* (`0.06` = 6 % of `size_px`), so they scale
//! with the text. Opacities and intensities are 0..=1, angles are degrees
//! clockwise from the right (y down: 45° points down and to the right).
//! Colours are straight sRGB RGBA. Values out of range are clamped when
//! drawn.

use crate::shape::bubble::BubbleSpec;

/// Underline and strikethrough (TX-008); thickness and position come from
/// the font, the colour from the fill.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Decoration {
    pub underline: bool,
    pub strike: bool,
}

/// A colour stop of a gradient.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientStop {
    /// 0..=1 along the gradient.
    pub at: f32,
    pub color: [u8; 4],
}

/// What a gradient spans (TX-014).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum FillExtent {
    /// The whole text box.
    #[default]
    Block,
    /// Each line's own box.
    Line,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LinearGradient {
    /// 0° runs left to right, 90° top to bottom.
    pub angle_deg: f32,
    /// 2..=8 stops; unsorted stops are sorted by position.
    pub stops: Vec<GradientStop>,
    pub extent: FillExtent,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RadialGradient {
    /// The centre as fractions (0..=1) of the extent's width and height.
    pub center: (f32, f32),
    /// The radius at the last stop: a fraction (0.1..=2) of half the
    /// extent's diagonal. The gradient is circular in layout space.
    pub spread: f32,
    pub stops: Vec<GradientStop>,
    pub extent: FillExtent,
}

/// The text fill (TX-004, TX-014). A gradient is mapped over the rest pose
/// of the text: every glyph keeps the colour it has at rest when it moves
/// (curve, per-unit motion).
#[derive(Debug, Clone, PartialEq)]
pub enum Fill {
    Solid([u8; 4]),
    Linear(LinearGradient),
    Radial(RadialGradient),
}

impl Default for Fill {
    fn default() -> Fill {
        Fill::Solid([255, 255, 255, 255])
    }
}

/// Where an outline sits relative to the glyph edge (TX-010).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum StrokePosition {
    /// Behind the fill, all of its width outside the letters.
    #[default]
    Outside,
    /// Over the fill, half its width over the letters' edge.
    Center,
    /// Over the fill, clipped to the letters.
    Inside,
}

/// An outline (TX-010).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stroke {
    pub color: [u8; 4],
    /// Fraction of the font size, 0..=0.3; 0 draws nothing.
    pub width: f32,
    pub opacity: f32,
    pub position: StrokePosition,
}

impl Default for Stroke {
    /// TX-010's defaults: black, 6 %, opaque, outside.
    fn default() -> Stroke {
        Stroke {
            color: [0, 0, 0, 255],
            width: 0.06,
            opacity: 1.0,
            position: StrokePosition::Outside,
        }
    }
}

/// A drop shadow (TX-011), cast by fill and outline together.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadow {
    pub color: [u8; 4],
    pub opacity: f32,
    /// Blur radius, fraction of the font size (0..=0.5).
    pub blur: f32,
    /// Offset, fraction of the font size (0..=1).
    pub distance: f32,
    /// Direction of the offset.
    pub angle_deg: f32,
    /// The background box or bubble casts the shadow too.
    pub include_box: bool,
}

impl Default for Shadow {
    /// TX-011's defaults: black at 50 %, blur 10 %, distance 8 %, 45°.
    fn default() -> Shadow {
        Shadow {
            color: [0, 0, 0, 255],
            opacity: 0.5,
            blur: 0.1,
            distance: 0.08,
            angle_deg: 45.0,
            include_box: false,
        }
    }
}

/// Which side of the glyph edge a glow lights (TX-012).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum GlowDirection {
    /// Around the letters, behind outline and fill.
    #[default]
    Outward,
    /// From the letters' edges inwards, clipped to them, over the fill.
    Inward,
    Both,
}

/// A glow (TX-012). It adds light to what lies beneath (premultiplied with
/// zero alpha), so it brightens dark backgrounds and saturates on white.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glow {
    pub color: [u8; 4],
    /// 0..=1.
    pub intensity: f32,
    /// Fraction of the font size (0..=1).
    pub radius: f32,
    /// Offset, fraction of the font size (0..=0.5).
    pub offset: f32,
    pub angle_deg: f32,
    pub direction: GlowDirection,
}

impl Default for Glow {
    /// TX-012's defaults: white, 60 %, radius 25 %, no offset, outward.
    fn default() -> Glow {
        Glow {
            color: [255, 255, 255, 255],
            intensity: 0.6,
            radius: 0.25,
            offset: 0.0,
            angle_deg: 45.0,
            direction: GlowDirection::Outward,
        }
    }
}

/// A border around a box or bubble, drawn inside its outline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Border {
    pub color: [u8; 4],
    /// Fraction of the font size (0..=0.2).
    pub width: f32,
}

/// How the background box follows the text (TX-013).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum BoxShape {
    /// One box around the text box.
    #[default]
    Block,
    /// One box per non-empty line.
    PerLine,
    /// One box per non-blank grapheme cluster.
    PerChar,
}

/// The vertical extent of per-line and per-character boxes (TX-013).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum BoxHeight {
    /// The line box.
    #[default]
    Full,
    /// A highlighter band: from half the x-height above the baseline to
    /// just below it (Per line and Per character only).
    LowerHalf,
}

/// The background box (TX-013). All boxes are one union shape, filled and
/// bordered once at the box opacity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextBox {
    pub color: [u8; 4],
    pub opacity: f32,
    pub shape: BoxShape,
    /// Padding, fractions of the font size (0..=1).
    pub pad_x: f32,
    pub pad_y: f32,
    /// 0..=1: 1 makes the radius half the box height (a pill).
    pub roundness: f32,
    pub height: BoxHeight,
    pub border: Option<Border>,
}

impl Default for TextBox {
    /// TX-013's defaults: black at 70 %, Block, padding 25 % × 15 %,
    /// roundness 20 %, full height, no border.
    fn default() -> TextBox {
        TextBox {
            color: [0, 0, 0, 255],
            opacity: 0.7,
            shape: BoxShape::Block,
            pad_x: 0.25,
            pad_y: 0.15,
            roundness: 0.2,
            height: BoxHeight::Full,
            border: None,
        }
    }
}

/// A bubble around the text (TX-055), drawn where the box is in the layer
/// order. The tail tip ([`crate::shape::bubble::Tail::tip`]) is in layout
/// px from the text box centre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextBubble {
    pub spec: BubbleSpec,
    pub fill: [u8; 4],
    pub opacity: f32,
    /// Padding, fractions of the font size.
    pub pad_x: f32,
    pub pad_y: f32,
    pub roundness: f32,
    pub border: Option<Border>,
}

impl Default for TextBubble {
    /// TX-055's defaults: white at 100 %, padding 45 % × 30 %, roundness
    /// 40 %, a dark border of 4 %; Rounded without a tail.
    fn default() -> TextBubble {
        TextBubble {
            spec: BubbleSpec::default(),
            fill: [255, 255, 255, 255],
            opacity: 1.0,
            pad_x: 0.45,
            pad_y: 0.3,
            roundness: 0.4,
            border: Some(Border {
                color: [30, 30, 36, 255],
                width: 0.04,
            }),
        }
    }
}
