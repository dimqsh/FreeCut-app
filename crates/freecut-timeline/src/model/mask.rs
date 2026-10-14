//! The shared shape-mask core (TR-017, TR-018, TR-020; CL-031): the shapes,
//! their geometry, feather and invert, and how several masks combine.
//! Colour masks (CL-031, slice K6) are the first user; clip masks (TR-017)
//! reuse the same type later and add Heart and Star to [`MaskShape`].
//!
//! Everything is fixed point, in the item's own frame (the clip's display
//! frame before its transform, or the canvas for an adjustment clip):
//! - lengths and positions in [`MASK_UNIT`]ths of the item's **shorter
//!   side** (10 000 = one shorter side), so a circle stays round on any
//!   aspect ratio and a mask means the same at every resolution;
//! - the centre is measured from the item's centre, +x right, +y down;
//! - rotation in 1/100 degree ([`Angle`]), positive clockwise on screen;
//! - feather, roundness and band width as [`PerMille`].
//!
//! The coverage maths (signed distances, smoothstep feather) lives in
//! `freecut-render`'s `shape::mask`; this module holds only values.

use super::transform::{Angle, PerMille};

/// One shorter side of the item, in mask units.
pub const MASK_UNIT: i32 = 10_000;
/// How far the centre may sit from the item's centre: ±3 shorter sides
/// (FreeCut default), so a shape can be pushed off the picture.
pub const MASK_CENTER_LIMIT: i32 = 3 * MASK_UNIT;
/// The largest width or height: 400 % of the shorter side (TR-017).
pub const MASK_SIZE_MAX: i32 = 4 * MASK_UNIT;
/// Rotation range: ±360° (TR-017).
pub const MASK_ROTATION_LIMIT: i32 = 36_000;
/// Feather and roundness top out at 100 % (TR-018).
pub const MASK_RATIO_MAX: u16 = 1000;
/// A band is at most 200 % of the shorter side wide (TR-017).
pub const MASK_BAND_MAX: u16 = 2000;
/// The share of the shorter side that a feather of 100 % spans (TR-018):
/// 25 %, centred on the edge.
pub const MASK_FEATHER_SPAN: f64 = 0.25;

/// A point or an extent in mask units: a centre (`x`, `y` from the item's
/// centre) or a size (`x` wide, `y` high).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct MaskPoint {
    pub x: i32,
    pub y: i32,
}

impl MaskPoint {
    pub const fn new(x: i32, y: i32) -> MaskPoint {
        MaskPoint { x, y }
    }

    /// As shorter sides, for the render and UI edge.
    pub fn to_f64(self) -> (f64, f64) {
        let u = f64::from(MASK_UNIT);
        (f64::from(self.x) / u, f64::from(self.y) / u)
    }

    /// The nearest point to (`x`, `y`) shorter sides, saturated at the
    /// `i32` range (the caller clamps into the field's range).
    pub fn from_f64(x: f64, y: f64) -> MaskPoint {
        let conv = |v: f64| {
            let v = (v * f64::from(MASK_UNIT)).round();
            let v = if v.is_nan() { 0.0 } else { v };
            // Saturated into i32 first, so the cast is exact.
            v.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
        };
        MaskPoint::new(conv(x), conv(y))
    }

    fn clamped(self, lo: i32, hi: i32) -> MaskPoint {
        MaskPoint::new(self.x.clamp(lo, hi), self.y.clamp(lo, hi))
    }
}

/// Which kind of shape a mask is, without its shape-only values: what the
/// shape picker offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MaskShapeKind {
    /// One straight edge through the centre; the side above it is kept.
    Linear,
    /// The strip between two parallel edges (a mirrored pair of linear
    /// edges).
    Band,
    /// An ellipse, a circle while its size is linked and square.
    Ellipse,
    /// A rectangle with rounded corners.
    Rectangle,
}

impl MaskShapeKind {
    /// Every kind, in picker order.
    pub const ALL: [MaskShapeKind; 4] = [
        MaskShapeKind::Linear,
        MaskShapeKind::Band,
        MaskShapeKind::Ellipse,
        MaskShapeKind::Rectangle,
    ];

    /// The stable name in project files.
    pub const fn as_str(self) -> &'static str {
        match self {
            MaskShapeKind::Linear => "linear",
            MaskShapeKind::Band => "band",
            MaskShapeKind::Ellipse => "ellipse",
            MaskShapeKind::Rectangle => "rectangle",
        }
    }

    pub fn parse(s: &str) -> Option<MaskShapeKind> {
        MaskShapeKind::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// Whether the shape has a width and height (Linear and Band don't:
    /// they reach across the whole item).
    pub const fn has_size(self) -> bool {
        matches!(self, MaskShapeKind::Ellipse | MaskShapeKind::Rectangle)
    }

    /// The default size of the shape (TR-017): 50 % for an ellipse,
    /// 60 % × 40 % for a rectangle (and the ellipse's for the shapes
    /// without a size, so switching to one starts there).
    pub const fn default_size(self) -> MaskPoint {
        match self {
            MaskShapeKind::Rectangle => MaskPoint::new(6_000, 4_000),
            _ => MaskPoint::new(5_000, 5_000),
        }
    }
}

/// A mask's shape and its shape-only values (TR-017).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MaskShape {
    Linear,
    /// `width`: the strip's width, 0–200 % of the shorter side.
    Band {
        width: PerMille,
    },
    Ellipse,
    /// `roundness`: the corner radius as a share of half the shorter
    /// side of the rectangle, 0–100 %.
    Rectangle {
        roundness: PerMille,
    },
}

impl MaskShape {
    pub const fn kind(self) -> MaskShapeKind {
        match self {
            MaskShape::Linear => MaskShapeKind::Linear,
            MaskShape::Band { .. } => MaskShapeKind::Band,
            MaskShape::Ellipse => MaskShapeKind::Ellipse,
            MaskShape::Rectangle { .. } => MaskShapeKind::Rectangle,
        }
    }

    /// The shape of `kind` with its defaults (a band 50 % wide, square
    /// corners).
    pub const fn of(kind: MaskShapeKind) -> MaskShape {
        match kind {
            MaskShapeKind::Linear => MaskShape::Linear,
            MaskShapeKind::Band => MaskShape::Band {
                width: PerMille::new(500),
            },
            MaskShapeKind::Ellipse => MaskShape::Ellipse,
            MaskShapeKind::Rectangle => MaskShape::Rectangle {
                roundness: PerMille::ZERO,
            },
        }
    }
}

/// A value of a mask that can take keyframes (TR-018's list): shape and
/// invert can't.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MaskParam {
    Center,
    Size,
    Rotation,
    Feather,
    /// Rectangles only.
    Roundness,
    /// Bands only.
    BandWidth,
}

impl MaskParam {
    /// Every value, in row order.
    pub const ALL: [MaskParam; 6] = [
        MaskParam::Center,
        MaskParam::Size,
        MaskParam::Rotation,
        MaskParam::Feather,
        MaskParam::Roundness,
        MaskParam::BandWidth,
    ];

    /// The stable name (the last part of a key path).
    pub const fn as_str(self) -> &'static str {
        match self {
            MaskParam::Center => "center",
            MaskParam::Size => "size",
            MaskParam::Rotation => "rotation",
            MaskParam::Feather => "feather",
            MaskParam::Roundness => "roundness",
            MaskParam::BandWidth => "band_width",
        }
    }

    pub fn parse(s: &str) -> Option<MaskParam> {
        MaskParam::ALL.into_iter().find(|p| p.as_str() == s)
    }
}

/// A value of [`MaskParam`], typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MaskValue {
    Point(MaskPoint),
    Angle(Angle),
    Ratio(PerMille),
}

/// One shape mask (TR-017, TR-018), in the item's own frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Mask {
    pub shape: MaskShape,
    /// From the item's centre, ±[`MASK_CENTER_LIMIT`].
    pub center: MaskPoint,
    /// Width and height, 0–[`MASK_SIZE_MAX`]. Kept for Linear and Band,
    /// which don't use it, so switching shape back keeps it (TR-017).
    pub size: MaskPoint,
    /// Whether width and height change together, keeping their ratio
    /// (TR-017's link toggle; a UI rule, the size is stored whole).
    pub linked: bool,
    /// ±360°.
    pub rotation: Angle,
    /// 0–100 % of [`MASK_FEATHER_SPAN`], centred on the edge (TR-018).
    pub feather: PerMille,
    /// Coverage flipped after feathering (TR-018).
    pub invert: bool,
}

impl Mask {
    /// A mask of `kind` with TR-017's defaults: centred, upright, no
    /// feather, not inverted; ellipses linked.
    pub const fn new(kind: MaskShapeKind) -> Mask {
        Mask {
            shape: MaskShape::of(kind),
            center: MaskPoint::new(0, 0),
            size: kind.default_size(),
            linked: matches!(kind, MaskShapeKind::Ellipse),
            rotation: Angle::ZERO,
            feather: PerMille::ZERO,
            invert: false,
        }
    }

    pub const fn kind(&self) -> MaskShapeKind {
        self.shape.kind()
    }

    /// The mask turned into `kind` (TR-017: centre, rotation, size,
    /// feather and invert stay; the new shape's own values start at their
    /// defaults). The same mask when it already is one.
    #[must_use]
    pub fn with_kind(&self, kind: MaskShapeKind) -> Mask {
        if self.kind() == kind {
            return *self;
        }
        Mask {
            shape: MaskShape::of(kind),
            ..*self
        }
    }

    /// Whether the shape encloses no area at all (TR-017's size 0): it
    /// hides everything, or shows everything inverted. A line always
    /// splits the item.
    pub fn is_empty_area(&self) -> bool {
        match self.shape {
            MaskShape::Linear => false,
            MaskShape::Band { width } => width.get() == 0,
            MaskShape::Ellipse | MaskShape::Rectangle { .. } => {
                self.size.x <= 0 || self.size.y <= 0
            }
        }
    }

    /// Every value limited to its range.
    #[must_use]
    pub fn clamped(&self) -> Mask {
        let ratio = |r: PerMille, max: u16| PerMille::new(r.get().min(max));
        Mask {
            shape: match self.shape {
                MaskShape::Band { width } => MaskShape::Band {
                    width: ratio(width, MASK_BAND_MAX),
                },
                MaskShape::Rectangle { roundness } => MaskShape::Rectangle {
                    roundness: ratio(roundness, MASK_RATIO_MAX),
                },
                other => other,
            },
            center: self.center.clamped(-MASK_CENTER_LIMIT, MASK_CENTER_LIMIT),
            size: self.size.clamped(0, MASK_SIZE_MAX),
            linked: self.linked,
            rotation: Angle::from_hundredths(
                self.rotation
                    .hundredths()
                    .clamp(-MASK_ROTATION_LIMIT, MASK_ROTATION_LIMIT),
            ),
            feather: ratio(self.feather, MASK_RATIO_MAX),
            invert: self.invert,
        }
    }

    /// Whether every value is in its range.
    pub fn is_valid(&self) -> bool {
        self.clamped() == *self
    }

    /// `param`'s value; `None` for a shape-only value the shape lacks.
    pub fn get(&self, param: MaskParam) -> Option<MaskValue> {
        Some(match (param, self.shape) {
            (MaskParam::Center, _) => MaskValue::Point(self.center),
            (MaskParam::Size, _) => MaskValue::Point(self.size),
            (MaskParam::Rotation, _) => MaskValue::Angle(self.rotation),
            (MaskParam::Feather, _) => MaskValue::Ratio(self.feather),
            (MaskParam::Roundness, MaskShape::Rectangle { roundness }) => {
                MaskValue::Ratio(roundness)
            }
            (MaskParam::BandWidth, MaskShape::Band { width }) => MaskValue::Ratio(width),
            (MaskParam::Roundness | MaskParam::BandWidth, _) => return None,
        })
    }

    /// The mask with `param` at `value`, clamped into its range; `None`
    /// when the shape lacks the value or `value` is of the wrong kind.
    pub fn with(&self, param: MaskParam, value: MaskValue) -> Option<Mask> {
        let mut out = *self;
        match (param, value, &mut out.shape) {
            (MaskParam::Center, MaskValue::Point(p), _) => out.center = p,
            (MaskParam::Size, MaskValue::Point(p), _) => out.size = p,
            (MaskParam::Rotation, MaskValue::Angle(a), _) => out.rotation = a,
            (MaskParam::Feather, MaskValue::Ratio(r), _) => out.feather = r,
            (MaskParam::Roundness, MaskValue::Ratio(r), MaskShape::Rectangle { roundness }) => {
                *roundness = r;
            }
            (MaskParam::BandWidth, MaskValue::Ratio(r), MaskShape::Band { width }) => *width = r,
            _ => return None,
        }
        Some(out.clamped())
    }
}

/// How a mask joins the coverage of the masks before it (TR-020), in list
/// order: Add takes the larger, Subtract cuts it out (`c·(1 − m)`),
/// Intersect keeps the overlap (the smaller).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MaskCombine {
    #[default]
    Add,
    Subtract,
    Intersect,
}

impl MaskCombine {
    pub const ALL: [MaskCombine; 3] = [
        MaskCombine::Add,
        MaskCombine::Subtract,
        MaskCombine::Intersect,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            MaskCombine::Add => "add",
            MaskCombine::Subtract => "subtract",
            MaskCombine::Intersect => "intersect",
        }
    }

    pub fn parse(s: &str) -> Option<MaskCombine> {
        MaskCombine::ALL.into_iter().find(|m| m.as_str() == s)
    }

    /// The coverage before the first mask of a list whose first mask has
    /// this mode (FreeCut default): nothing for Add, so the first mask is
    /// the area; everything for Subtract and Intersect, so a list that
    /// starts by cutting out keeps the rest of the item.
    pub const fn start(self) -> f32 {
        match self {
            MaskCombine::Add => 0.0,
            MaskCombine::Subtract | MaskCombine::Intersect => 1.0,
        }
    }

    /// `acc` joined with the next mask's coverage `m`.
    pub fn join(self, acc: f32, m: f32) -> f32 {
        match self {
            MaskCombine::Add => acc.max(m),
            MaskCombine::Subtract => acc * (1.0 - m),
            MaskCombine::Intersect => acc.min(m),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_tr_017() {
        let e = Mask::new(MaskShapeKind::Ellipse);
        assert_eq!(e.size, MaskPoint::new(5_000, 5_000));
        assert!(e.linked && !e.invert && e.is_valid());
        let r = Mask::new(MaskShapeKind::Rectangle);
        assert_eq!(r.size, MaskPoint::new(6_000, 4_000));
        assert!(!r.linked);
        assert_eq!(
            Mask::new(MaskShapeKind::Band).shape,
            MaskShape::Band {
                width: PerMille::new(500)
            }
        );
        for k in MaskShapeKind::ALL {
            assert_eq!(MaskShapeKind::parse(k.as_str()), Some(k));
            assert_eq!(Mask::new(k).kind(), k);
        }
        for p in MaskParam::ALL {
            assert_eq!(MaskParam::parse(p.as_str()), Some(p));
        }
        for m in MaskCombine::ALL {
            assert_eq!(MaskCombine::parse(m.as_str()), Some(m));
        }
    }

    #[test]
    fn changing_shape_keeps_the_common_values() {
        let m = Mask {
            center: MaskPoint::new(1_200, -300),
            rotation: Angle::from_degrees(30),
            feather: PerMille::new(400),
            invert: true,
            shape: MaskShape::Rectangle {
                roundness: PerMille::new(700),
            },
            ..Mask::new(MaskShapeKind::Rectangle)
        };
        let e = m.with_kind(MaskShapeKind::Ellipse);
        assert_eq!(
            (e.center, e.size, e.rotation, e.feather, e.invert),
            (m.center, m.size, m.rotation, m.feather, m.invert)
        );
        assert_eq!(e.get(MaskParam::Roundness), None);
        let back = e.with_kind(MaskShapeKind::Rectangle);
        assert_eq!(
            back.get(MaskParam::Roundness),
            Some(MaskValue::Ratio(PerMille::ZERO)),
            "shape-only values start again at their defaults"
        );
    }

    #[test]
    fn values_are_clamped_and_shape_only_values_need_their_shape() {
        let m = Mask::new(MaskShapeKind::Ellipse);
        let wild = m
            .with(
                MaskParam::Center,
                MaskValue::Point(MaskPoint::new(99_999, -99_999)),
            )
            .unwrap()
            .with(
                MaskParam::Size,
                MaskValue::Point(MaskPoint::new(-5, 90_000)),
            )
            .unwrap()
            .with(
                MaskParam::Rotation,
                MaskValue::Angle(Angle::from_degrees(900)),
            )
            .unwrap()
            .with(MaskParam::Feather, MaskValue::Ratio(PerMille::new(5_000)))
            .unwrap();
        assert_eq!(wild.center, MaskPoint::new(30_000, -30_000));
        assert_eq!(wild.size, MaskPoint::new(0, 40_000));
        assert_eq!(wild.rotation, Angle::from_degrees(360));
        assert_eq!(wild.feather, PerMille::ONE);
        assert!(wild.is_empty_area());
        assert_eq!(
            m.with(MaskParam::BandWidth, MaskValue::Ratio(PerMille::ONE)),
            None
        );
        assert_eq!(
            m.with(MaskParam::Center, MaskValue::Ratio(PerMille::ONE)),
            None,
            "wrong kind"
        );
        let band = Mask::new(MaskShapeKind::Band)
            .with(MaskParam::BandWidth, MaskValue::Ratio(PerMille::new(9_000)))
            .unwrap();
        assert_eq!(
            band.get(MaskParam::BandWidth),
            Some(MaskValue::Ratio(PerMille::new(2_000)))
        );
        assert!(!Mask::new(MaskShapeKind::Linear).is_empty_area());
    }

    #[test]
    fn combine_modes_follow_tr_020() {
        assert_eq!(MaskCombine::Add.join(0.25, 0.5), 0.5);
        assert_eq!(MaskCombine::Subtract.join(1.0, 0.25), 0.75);
        assert_eq!(MaskCombine::Intersect.join(0.25, 0.5), 0.25);
        assert_eq!(MaskCombine::Add.start(), 0.0);
        assert_eq!(MaskCombine::Subtract.start(), 1.0);
    }
}
