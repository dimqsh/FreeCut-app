//! The parts of a [`TextStyle`](super::TextStyle) beyond font and size:
//! decoration and case (TX-008), fills (TX-004, TX-014), and the optional
//! layers outline, shadow, glow and background box (TX-010 to TX-013),
//! each kept in a [`Layer`] so switching it off keeps its settings (TX-054).
//!
//! Lengths relative to the font size are per-mille of the size; angles are
//! [`DeciDegrees`]. Every `clamped` puts a value into its spec range, and
//! `is_valid` means `clamped` changes nothing.

use super::Rgba8;
use super::units::{DeciDegrees, clamp_pm};
use crate::model::PerMille;

/// An optional style layer (TX-054): switched on or off, with its settings
/// kept either way.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Layer<T> {
    pub on: bool,
    pub value: T,
}

impl<T> Layer<T> {
    pub const fn on(value: T) -> Layer<T> {
        Layer { on: true, value }
    }

    pub const fn off(value: T) -> Layer<T> {
        Layer { on: false, value }
    }

    /// The settings while the layer is on.
    pub fn active(&self) -> Option<&T> {
        self.on.then_some(&self.value)
    }

    pub(crate) fn map(self, f: impl FnOnce(T) -> T) -> Layer<T> {
        Layer {
            on: self.on,
            value: f(self.value),
        }
    }
}

/// Underline and strikethrough (TX-008).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Decoration {
    pub underline: bool,
    pub strike: bool,
}

/// The display case transform (TX-008). Only display changes; the stored
/// content stays as typed. See [`case`](super::case) for the mapping.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum CaseTransform {
    #[default]
    AsTyped,
    Upper,
    Lower,
    /// The first letter of every word capitalized, the rest lowercase.
    Title,
    /// The first letter of each sentence capitalized, the rest lowercase.
    Sentence,
}

/// One colour stop of a gradient (TX-014).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GradientStop {
    /// 0–1000 per-mille along the gradient.
    pub at: PerMille,
    pub color: Rgba8,
}

/// Why a stop list was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GradientError {
    #[error("a gradient has 2 to 8 stops, not {0}")]
    StopCount(usize),
    #[error("a stop position is above 100 %")]
    StopPosition,
}

/// The 2–8 stops of a gradient (TX-014), sorted by position (stops at the
/// same position keep their order and make a hard edge). Valid by
/// construction.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GradientStops(Vec<GradientStop>);

impl GradientStops {
    pub const MIN: usize = 2;
    pub const MAX: usize = 8;

    /// `stops`, sorted by position; refused unless there are 2–8, each at
    /// most 100 %.
    pub fn new(mut stops: Vec<GradientStop>) -> Result<GradientStops, GradientError> {
        if !(Self::MIN..=Self::MAX).contains(&stops.len()) {
            return Err(GradientError::StopCount(stops.len()));
        }
        if stops.iter().any(|s| s.at > PerMille::ONE) {
            return Err(GradientError::StopPosition);
        }
        stops.sort_by_key(|s| s.at);
        Ok(GradientStops(stops))
    }

    /// `from` at 0 % to `to` at 100 %.
    pub fn two(from: Rgba8, to: Rgba8) -> GradientStops {
        GradientStops(vec![
            GradientStop {
                at: PerMille::ZERO,
                color: from,
            },
            GradientStop {
                at: PerMille::ONE,
                color: to,
            },
        ])
    }

    pub fn as_slice(&self) -> &[GradientStop] {
        &self.0
    }

    /// The editor's reverse button: every stop mirrored.
    pub fn reversed(&self) -> GradientStops {
        let mut out: Vec<GradientStop> = self
            .0
            .iter()
            .rev()
            .map(|s| GradientStop {
                at: PerMille::new(PerMille::ONE.get() - s.at.get()),
                color: s.color,
            })
            .collect();
        // Mirroring a sorted list gives a sorted list; keep the guarantee
        // explicit anyway.
        out.sort_by_key(|s| s.at);
        GradientStops(out)
    }

    /// The same stops with stop `index` recoloured; `None` when there is no
    /// such stop.
    pub fn with_color(&self, index: usize, color: Rgba8) -> Option<GradientStops> {
        let mut out = self.clone();
        out.0.get_mut(index)?.color = color;
        Some(out)
    }

    /// The first stop's colour: the solid colour a gradient falls back to.
    pub fn first_color(&self) -> Rgba8 {
        // Never empty: construction requires at least two stops.
        self.0.first().map_or(Rgba8::WHITE, |s| s.color)
    }
}

/// What a gradient spans (TX-014).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum FillExtent {
    /// The whole text block.
    #[default]
    Block,
    /// Each line on its own.
    Line,
}

/// A radial gradient's centre, in per-mille of the extent's bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FillCenter {
    pub x: PerMille,
    pub y: PerMille,
}

impl FillCenter {
    pub const MIDDLE: FillCenter = FillCenter {
        x: PerMille::new(500),
        y: PerMille::new(500),
    };

    fn clamped(self) -> FillCenter {
        FillCenter {
            x: clamp_pm(self.x, 0, 1000),
            y: clamp_pm(self.y, 0, 1000),
        }
    }
}

/// A linear gradient fill (TX-014).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LinearGradient {
    /// 0° runs left to right, 90° (the default) top to bottom.
    pub angle: DeciDegrees,
    pub stops: GradientStops,
    pub extent: FillExtent,
}

/// A radial gradient fill (TX-014), circular in canvas space.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RadialGradient {
    pub center: FillCenter,
    /// The radius at the last stop: 100–2000 per-mille of half the extent's
    /// diagonal, default 1000.
    pub spread: PerMille,
    pub stops: GradientStops,
    pub extent: FillExtent,
}

impl RadialGradient {
    pub const MIN_SPREAD: PerMille = PerMille::new(100);
    pub const MAX_SPREAD: PerMille = PerMille::new(2000);
}

/// Which kind of fill (TX-014's Solid / Linear / Radial choice).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FillKind {
    Solid,
    LinearGradient,
    RadialGradient,
}

/// How glyphs are filled (TX-004, TX-014). Image and word-colour fills are
/// later (TX-046, TX-047).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Fill {
    Solid(Rgba8),
    LinearGradient(LinearGradient),
    RadialGradient(RadialGradient),
}

impl Fill {
    pub fn kind(&self) -> FillKind {
        match self {
            Fill::Solid(_) => FillKind::Solid,
            Fill::LinearGradient(_) => FillKind::LinearGradient,
            Fill::RadialGradient(_) => FillKind::RadialGradient,
        }
    }

    /// The solid colour, if this is a solid fill.
    pub fn solid(&self) -> Option<Rgba8> {
        match self {
            Fill::Solid(c) => Some(*c),
            _ => None,
        }
    }

    pub fn stops(&self) -> Option<&GradientStops> {
        match self {
            Fill::Solid(_) => None,
            Fill::LinearGradient(g) => Some(&g.stops),
            Fill::RadialGradient(g) => Some(&g.stops),
        }
    }

    /// Every colour the fill shows: one for solid, each stop's otherwise.
    pub fn colors(&self) -> Vec<Rgba8> {
        match self.stops() {
            None => self.solid().into_iter().collect(),
            Some(stops) => stops.as_slice().iter().map(|s| s.color).collect(),
        }
    }

    /// This fill switched to `kind` (TX-014): linear and radial keep their
    /// stops and extent; a solid colour `c` becomes a gradient from `c` to
    /// `c` at half brightness (FreeCut default); a gradient becomes solid in
    /// its first stop's colour.
    pub fn with_kind(&self, kind: FillKind) -> Fill {
        if self.kind() == kind {
            return self.clone();
        }
        let (stops, extent) = match self {
            Fill::Solid(c) => (
                GradientStops::two(*c, Rgba8::new(c.r / 2, c.g / 2, c.b / 2, c.a)),
                FillExtent::Block,
            ),
            Fill::LinearGradient(g) => (g.stops.clone(), g.extent),
            Fill::RadialGradient(g) => (g.stops.clone(), g.extent),
        };
        match kind {
            FillKind::Solid => Fill::Solid(stops.first_color()),
            FillKind::LinearGradient => Fill::LinearGradient(LinearGradient {
                angle: DeciDegrees::DOWN,
                stops,
                extent,
            }),
            FillKind::RadialGradient => Fill::RadialGradient(RadialGradient {
                center: FillCenter::MIDDLE,
                spread: PerMille::ONE,
                stops,
                extent,
            }),
        }
    }

    pub(crate) fn clamped(&self) -> Fill {
        match self {
            Fill::RadialGradient(g) => Fill::RadialGradient(RadialGradient {
                center: g.center.clamped(),
                spread: clamp_pm(
                    g.spread,
                    RadialGradient::MIN_SPREAD.get(),
                    RadialGradient::MAX_SPREAD.get(),
                ),
                ..g.clone()
            }),
            other => other.clone(),
        }
    }
}

impl Default for Fill {
    fn default() -> Fill {
        Fill::Solid(Rgba8::WHITE)
    }
}

impl From<Rgba8> for Fill {
    fn from(color: Rgba8) -> Fill {
        Fill::Solid(color)
    }
}

/// Where an outline sits on the glyph edge (TX-010).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum StrokePosition {
    #[default]
    Outside,
    Center,
    Inside,
}

/// The outline (TX-010).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Stroke {
    pub color: Rgba8,
    /// 0–300 per-mille of the font size; 0 draws nothing.
    pub width: PerMille,
    pub opacity: PerMille,
    pub position: StrokePosition,
}

impl Stroke {
    pub const MAX_WIDTH: PerMille = PerMille::new(300);

    pub(crate) fn clamped(self) -> Stroke {
        Stroke {
            width: clamp_pm(self.width, 0, Self::MAX_WIDTH.get()),
            opacity: clamp_pm(self.opacity, 0, 1000),
            ..self
        }
    }
}

impl Default for Stroke {
    /// Black, 6 % of the size, opaque, outside (TX-010).
    fn default() -> Stroke {
        Stroke {
            color: Rgba8::BLACK,
            width: PerMille::new(60),
            opacity: PerMille::ONE,
            position: StrokePosition::Outside,
        }
    }
}

/// The drop shadow (TX-011).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Shadow {
    pub color: Rgba8,
    pub opacity: PerMille,
    /// 0–500 per-mille of the font size.
    pub blur: PerMille,
    /// 0–1000 per-mille of the font size.
    pub distance: PerMille,
    pub angle: DeciDegrees,
    /// The background box or bubble casts a shadow too.
    pub include_box: bool,
}

impl Shadow {
    pub const MAX_BLUR: PerMille = PerMille::new(500);
    pub const MAX_DISTANCE: PerMille = PerMille::new(1000);

    pub(crate) fn clamped(self) -> Shadow {
        Shadow {
            opacity: clamp_pm(self.opacity, 0, 1000),
            blur: clamp_pm(self.blur, 0, Self::MAX_BLUR.get()),
            distance: clamp_pm(self.distance, 0, Self::MAX_DISTANCE.get()),
            ..self
        }
    }
}

impl Default for Shadow {
    /// Black at 50 %, blur 10 %, distance 8 %, 45° (TX-011).
    fn default() -> Shadow {
        Shadow {
            color: Rgba8::BLACK,
            opacity: PerMille::new(500),
            blur: PerMille::new(100),
            distance: PerMille::new(80),
            angle: DeciDegrees::DOWN_RIGHT,
            include_box: false,
        }
    }
}

/// Which way a glow shines (TX-012).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum GlowDirection {
    #[default]
    Outward,
    /// From the glyph edges toward the middle, clipped to the glyphs.
    Inward,
    Both,
}

/// The glow (TX-012).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Glow {
    pub color: Rgba8,
    pub intensity: PerMille,
    /// 0–1000 per-mille of the font size.
    pub radius: PerMille,
    /// 0–500 per-mille of the font size.
    pub offset: PerMille,
    pub angle: DeciDegrees,
    pub direction: GlowDirection,
}

impl Glow {
    pub const MAX_RADIUS: PerMille = PerMille::new(1000);
    pub const MAX_OFFSET: PerMille = PerMille::new(500);

    pub(crate) fn clamped(self) -> Glow {
        Glow {
            intensity: clamp_pm(self.intensity, 0, 1000),
            radius: clamp_pm(self.radius, 0, Self::MAX_RADIUS.get()),
            offset: clamp_pm(self.offset, 0, Self::MAX_OFFSET.get()),
            ..self
        }
    }
}

impl Default for Glow {
    /// White, 60 %, radius 25 %, no offset (aimed at 45°), outward (TX-012).
    fn default() -> Glow {
        Glow {
            color: Rgba8::WHITE,
            intensity: PerMille::new(600),
            radius: PerMille::new(250),
            offset: PerMille::ZERO,
            angle: DeciDegrees::DOWN_RIGHT,
            direction: GlowDirection::Outward,
        }
    }
}

/// A border around a box or bubble (TX-013, TX-055).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Border {
    pub color: Rgba8,
    /// 0–200 per-mille of the font size.
    pub width: PerMille,
}

impl Border {
    pub const MAX_WIDTH: PerMille = PerMille::new(200);

    pub(crate) fn clamped(self) -> Border {
        Border {
            width: clamp_pm(self.width, 0, Self::MAX_WIDTH.get()),
            ..self
        }
    }
}

/// How a background box covers the text (TX-013).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum BoxShape {
    #[default]
    Block,
    PerLine,
    /// One box per grapheme cluster.
    PerChar,
}

/// How tall each box is (TX-013). `LowerHalf` applies to `PerLine` and
/// `PerChar`; with `Block` it is kept but drawn as `Full`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum BoxHeight {
    #[default]
    Full,
    /// From just below the baseline to half the x-height.
    LowerHalf,
}

/// The background box (TX-013).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextBox {
    pub color: Rgba8,
    pub opacity: PerMille,
    pub shape: BoxShape,
    /// 0–1000 per-mille of the font size.
    pub pad_x: PerMille,
    pub pad_y: PerMille,
    /// 0–1000; at 1000 the radius is half the box height.
    pub roundness: PerMille,
    pub height: BoxHeight,
    pub border: Layer<Border>,
}

impl TextBox {
    pub(crate) fn clamped(self) -> TextBox {
        TextBox {
            opacity: clamp_pm(self.opacity, 0, 1000),
            pad_x: clamp_pm(self.pad_x, 0, 1000),
            pad_y: clamp_pm(self.pad_y, 0, 1000),
            roundness: clamp_pm(self.roundness, 0, 1000),
            border: self.border.map(Border::clamped),
            ..self
        }
    }
}

impl Default for TextBox {
    /// Black at 70 %, Block, padding 25 % × 15 %, roundness 20 %, full
    /// height, border off (white, 3 %) (TX-013).
    fn default() -> TextBox {
        TextBox {
            color: Rgba8::BLACK,
            opacity: PerMille::new(700),
            shape: BoxShape::Block,
            pad_x: PerMille::new(250),
            pad_y: PerMille::new(150),
            roundness: PerMille::new(200),
            height: BoxHeight::Full,
            border: Layer::off(Border {
                color: Rgba8::WHITE,
                width: PerMille::new(30),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stop(at: u16, c: u8) -> GradientStop {
        GradientStop {
            at: PerMille::new(at),
            color: Rgba8::new(c, c, c, 255),
        }
    }

    #[test]
    fn stops_are_sorted_and_counted() {
        assert_eq!(
            GradientStops::new(vec![stop(0, 0)]),
            Err(GradientError::StopCount(1))
        );
        assert_eq!(
            GradientStops::new(vec![stop(0, 0); 9]),
            Err(GradientError::StopCount(9))
        );
        assert_eq!(
            GradientStops::new(vec![stop(0, 0), stop(1001, 0)]),
            Err(GradientError::StopPosition)
        );
        let s = GradientStops::new(vec![stop(800, 1), stop(200, 2), stop(200, 3)]).unwrap();
        assert_eq!(s.as_slice(), &[stop(200, 2), stop(200, 3), stop(800, 1)]);
        assert_eq!(
            s.reversed().as_slice(),
            &[stop(200, 1), stop(800, 3), stop(800, 2)]
        );
        assert_eq!(s.with_color(9, Rgba8::WHITE), None);
    }

    #[test]
    fn switching_fill_kinds_keeps_stops() {
        let solid = Fill::Solid(Rgba8::new(200, 100, 50, 255));
        let linear = solid.with_kind(FillKind::LinearGradient);
        let stops = linear.stops().unwrap().clone();
        assert_eq!(
            stops,
            GradientStops::two(Rgba8::new(200, 100, 50, 255), Rgba8::new(100, 50, 25, 255))
        );
        let radial = linear.with_kind(FillKind::RadialGradient);
        assert_eq!(radial.stops(), Some(&stops));
        assert_eq!(
            radial.with_kind(FillKind::Solid),
            Fill::Solid(Rgba8::new(200, 100, 50, 255))
        );
        assert_eq!(solid.with_kind(FillKind::Solid), solid);
    }

    #[test]
    fn layers_keep_their_value_when_off() {
        let l = Layer::on(Stroke::default());
        assert_eq!(l.active(), Some(&Stroke::default()));
        let off = Layer { on: false, ..l };
        assert_eq!(off.active(), None);
        assert_eq!(off.value, Stroke::default());
    }
}
