//! Colour masks (CL-031): shape masks that limit where the grade applies.
//! The shape, feather and invert are TR's [`Mask`]; a colour mask adds
//! Expand and its combine mode, and an id that keys and edits name it by,
//! so reordering never moves a key to another mask.

use crate::model::keys::PropValue;
use crate::model::mask::{Mask, MaskCombine, MaskParam, MaskValue};

/// At most 8 masks per stack (CL-031, the same cap as TR-020).
pub const MAX_COLOR_MASKS: usize = 8;

/// Expand's range (CL-031): −100..=+100, ±10 % of the shorter side.
pub const EXPAND_LIMIT: i16 = 100;

/// One colour mask (CL-031).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColorMask {
    /// Unique within the stack, never 0.
    pub id: u16,
    pub mask: Mask,
    /// Grows (+) or shrinks (−) the edge, as an offset on the signed
    /// distance before feathering.
    pub expand: i16,
    pub mode: MaskCombine,
}

impl ColorMask {
    /// A new mask named `id`: Expand 0, combined by Add.
    pub fn new(id: u16, mask: Mask) -> ColorMask {
        ColorMask {
            id,
            mask: mask.clamped(),
            expand: 0,
            mode: MaskCombine::Add,
        }
    }

    /// Every value limited to its range.
    #[must_use]
    pub fn clamped(&self) -> ColorMask {
        ColorMask {
            mask: self.mask.clamped(),
            expand: self.expand.clamp(-EXPAND_LIMIT, EXPAND_LIMIT),
            ..*self
        }
    }

    /// `param`'s value as a keyframe holds it; `None` for a shape-only
    /// value the shape lacks.
    pub fn get(&self, param: ColorMaskParam) -> Option<PropValue> {
        match param {
            ColorMaskParam::Expand => Some(PropValue::Level(self.expand)),
            ColorMaskParam::Shape(p) => self.mask.get(p).map(to_prop),
        }
    }

    /// The mask with `param` at `value`, clamped; `None` when it lacks the
    /// value or `value` is of the wrong kind.
    pub fn with(&self, param: ColorMaskParam, value: PropValue) -> Option<ColorMask> {
        match (param, value) {
            (ColorMaskParam::Expand, PropValue::Level(v)) => Some(ColorMask {
                expand: v.clamp(-EXPAND_LIMIT, EXPAND_LIMIT),
                ..*self
            }),
            (ColorMaskParam::Expand, _) => None,
            (ColorMaskParam::Shape(p), v) => Some(ColorMask {
                mask: self.mask.with(p, from_prop(v)?)?,
                ..*self
            }),
        }
    }
}

/// A keyable value of a colour mask: one of the shape's (TR-018) or
/// Expand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ColorMaskParam {
    Shape(MaskParam),
    Expand,
}

impl ColorMaskParam {
    /// Every value, in row order.
    pub const ALL: [ColorMaskParam; 7] = [
        ColorMaskParam::Shape(MaskParam::Center),
        ColorMaskParam::Shape(MaskParam::Size),
        ColorMaskParam::Shape(MaskParam::Rotation),
        ColorMaskParam::Shape(MaskParam::Feather),
        ColorMaskParam::Shape(MaskParam::Roundness),
        ColorMaskParam::Shape(MaskParam::BandWidth),
        ColorMaskParam::Expand,
    ];

    /// The stable name (the last part of a key path).
    pub const fn as_str(self) -> &'static str {
        match self {
            ColorMaskParam::Shape(p) => p.as_str(),
            ColorMaskParam::Expand => "expand",
        }
    }

    pub fn parse(s: &str) -> Option<ColorMaskParam> {
        ColorMaskParam::ALL.into_iter().find(|p| p.as_str() == s)
    }
}

fn to_prop(v: MaskValue) -> PropValue {
    match v {
        MaskValue::Point(p) => PropValue::MaskPoint(p),
        MaskValue::Angle(a) => PropValue::Angle(a),
        MaskValue::Ratio(r) => PropValue::Ratio(r),
    }
}

fn from_prop(v: PropValue) -> Option<MaskValue> {
    Some(match v {
        PropValue::MaskPoint(p) => MaskValue::Point(p),
        PropValue::Angle(a) => MaskValue::Angle(a),
        PropValue::Ratio(r) => MaskValue::Ratio(r),
        _ => return None,
    })
}

/// The smallest id from 1 up that `masks` doesn't use; `None` when all are
/// taken (never with at most [`MAX_COLOR_MASKS`] masks).
pub(crate) fn free_id(masks: &[ColorMask]) -> Option<u16> {
    (1..=u16::MAX).find(|id| masks.iter().all(|m| m.id != *id))
}

/// Whether `masks` is a valid list: at most 8, ids nonzero and unique,
/// values in range.
pub(crate) fn masks_valid(masks: &[ColorMask]) -> bool {
    masks.len() <= MAX_COLOR_MASKS
        && masks
            .iter()
            .enumerate()
            .all(|(i, m)| m.id != 0 && masks[..i].iter().all(|o| o.id != m.id))
        && masks.iter().all(|m| m.clamped() == *m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::mask::{MaskPoint, MaskShapeKind};
    use crate::model::transform::PerMille;

    #[test]
    fn values_go_through_prop_values() {
        let m = ColorMask::new(3, Mask::new(MaskShapeKind::Ellipse));
        let c = ColorMaskParam::Shape(MaskParam::Center);
        let moved = m
            .with(c, PropValue::MaskPoint(MaskPoint::new(1_000, 2_000)))
            .unwrap();
        assert_eq!(moved.mask.center, MaskPoint::new(1_000, 2_000));
        assert_eq!(
            moved.get(c),
            Some(PropValue::MaskPoint(MaskPoint::new(1_000, 2_000)))
        );
        let e = m
            .with(ColorMaskParam::Expand, PropValue::Level(400))
            .unwrap();
        assert_eq!(e.expand, 100);
        assert_eq!(
            m.with(ColorMaskParam::Expand, PropValue::Ratio(PerMille::ONE)),
            None
        );
        assert_eq!(m.get(ColorMaskParam::Shape(MaskParam::BandWidth)), None);
        for p in ColorMaskParam::ALL {
            assert_eq!(ColorMaskParam::parse(p.as_str()), Some(p));
        }
    }

    #[test]
    fn ids_are_reused_from_the_smallest_free_one() {
        let e = Mask::new(MaskShapeKind::Ellipse);
        let list = [ColorMask::new(1, e), ColorMask::new(3, e)];
        assert_eq!(free_id(&list), Some(2));
        assert!(masks_valid(&list));
        assert!(!masks_valid(&[ColorMask::new(1, e), ColorMask::new(1, e)]));
        assert!(!masks_valid(&[ColorMask::new(0, e)]));
    }
}
