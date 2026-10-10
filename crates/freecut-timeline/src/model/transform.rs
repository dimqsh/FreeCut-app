//! Clip transform (TR-001, TR-003) and opacity (TR-015).
//!
//! FreeCut stores these static values as fixed-point integers so clips stay
//! exactly comparable and project files stable; floats appear only where the
//! render plan builds the matrix. Units:
//! - position: millionths of the canvas width (x) and height (y), measured
//!   from the canvas centre to the clip's anchor (its centre), +x right,
//!   +y down;
//! - scale: basis points (1/100 %) of the clip's fitted size (TR-002);
//! - rotation: 1/100 degree, positive clockwise, not wrapped;
//! - opacity: per-mille.

/// A ratio in thousandths (1000 = 100 %).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PerMille(u16);

impl PerMille {
    pub const ZERO: PerMille = PerMille(0);
    /// 100 %.
    pub const ONE: PerMille = PerMille(1000);

    pub const fn new(per_mille: u16) -> PerMille {
        PerMille(per_mille)
    }

    pub const fn get(self) -> u16 {
        self.0
    }

    /// The ratio as a factor (1000 → 1.0), for the render and UI edge.
    pub fn as_f64(self) -> f64 {
        f64::from(self.0) / 1000.0
    }
}

/// A point on the canvas relative to its centre, in millionths of the
/// canvas width (x) and height (y). `x = 500_000` is the right edge.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct CanvasPos {
    pub x: i32,
    pub y: i32,
}

impl CanvasPos {
    pub const CENTER: CanvasPos = CanvasPos { x: 0, y: 0 };
    /// One full canvas width or height.
    pub const UNIT: i32 = 1_000_000;
    /// ±4 canvas widths or heights (TR-001, FreeCut default).
    pub const LIMIT: i32 = 4 * Self::UNIT;

    pub const fn new(x: i32, y: i32) -> CanvasPos {
        CanvasPos { x, y }
    }

    /// The offset in canvas pixels for a `width` × `height` canvas.
    pub fn to_px(self, width: u32, height: u32) -> (f64, f64) {
        let unit = f64::from(Self::UNIT);
        (
            f64::from(self.x) * f64::from(width) / unit,
            f64::from(self.y) * f64::from(height) / unit,
        )
    }

    /// The stored position nearest an offset of (`x`, `y`) canvas pixels on
    /// a `width` × `height` canvas, clamped to ±4 canvas sizes. For the UI
    /// edge (inspector fields, handle drags, nudges).
    pub fn from_px(x: f64, y: f64, width: u32, height: u32) -> CanvasPos {
        let conv = |px: f64, side: u32| {
            let v = (px * f64::from(Self::UNIT) / f64::from(side.max(1))).round();
            let v = if v.is_nan() { 0.0 } else { v };
            // In range after clamping, so the cast is exact.
            v.clamp(f64::from(-Self::LIMIT), f64::from(Self::LIMIT)) as i32
        };
        CanvasPos {
            x: conv(x, width),
            y: conv(y, height),
        }
    }

    fn clamped(self) -> CanvasPos {
        CanvasPos {
            x: self.x.clamp(-Self::LIMIT, Self::LIMIT),
            y: self.y.clamp(-Self::LIMIT, Self::LIMIT),
        }
    }
}

/// Uniform scale in basis points of the fitted size: 10,000 = 100 %.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Scale(i32);

impl Scale {
    /// 1 % (TR-001).
    pub const MIN: Scale = Scale(100);
    /// 1000 % (TR-001).
    pub const MAX: Scale = Scale(100_000);
    /// 100 %.
    pub const ONE: Scale = Scale(10_000);

    pub const fn from_basis_points(bp: i32) -> Scale {
        Scale(bp)
    }

    /// Whole percent.
    pub const fn from_percent(percent: i32) -> Scale {
        Scale(percent.saturating_mul(100))
    }

    pub const fn basis_points(self) -> i32 {
        self.0
    }

    /// The scale factor (10,000 → 1.0), for the render and UI edge.
    pub fn factor(self) -> f64 {
        f64::from(self.0) / 10_000.0
    }

    /// Clamped to 1 %–1000 %; zero and negative values become 1 % (TR-001).
    pub fn clamped(self) -> Scale {
        Scale(self.0.clamp(Self::MIN.0, Self::MAX.0))
    }
}

impl Default for Scale {
    fn default() -> Scale {
        Scale::ONE
    }
}

/// An angle in 1/100 degree, positive clockwise on screen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Angle(i32);

impl Angle {
    pub const ZERO: Angle = Angle(0);
    /// ±3600° (ten turns, TR-001).
    pub const LIMIT: Angle = Angle(360_000);

    pub const fn from_hundredths(hundredths: i32) -> Angle {
        Angle(hundredths)
    }

    /// Whole degrees.
    pub const fn from_degrees(degrees: i32) -> Angle {
        Angle(degrees.saturating_mul(100))
    }

    pub const fn hundredths(self) -> i32 {
        self.0
    }

    /// Degrees, for the render and UI edge.
    pub fn degrees(self) -> f64 {
        f64::from(self.0) / 100.0
    }

    /// Clamped to ±3600°, never wrapped (370° stays 370°).
    pub fn clamped(self) -> Angle {
        Angle(self.0.clamp(-Self::LIMIT.0, Self::LIMIT.0))
    }
}

/// Where and how a visual clip sits on the canvas (TR-001, TR-003) and how
/// opaque it is (TR-015). Audio clips keep the default.
///
/// Rendering order inside the clip (TR shared conventions): flip about the
/// clip centre, scale and rotate about the anchor (the centre), then move
/// the anchor to `position`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Transform {
    pub position: CanvasPos,
    pub scale: Scale,
    pub rotation: Angle,
    pub flip_h: bool,
    pub flip_v: bool,
    pub opacity: PerMille,
}

impl Transform {
    pub const IDENTITY: Transform = Transform {
        position: CanvasPos::CENTER,
        scale: Scale::ONE,
        rotation: Angle::ZERO,
        flip_h: false,
        flip_v: false,
        opacity: PerMille::ONE,
    };

    /// Every value inside its range.
    pub fn is_valid(&self) -> bool {
        self.clamped() == *self
    }

    /// The same transform with every value clamped into its range (TR-001,
    /// TR-015), for edits and for loaders of hand-edited files.
    pub fn clamped(&self) -> Transform {
        Transform {
            position: self.position.clamped(),
            scale: self.scale.clamped(),
            rotation: self.rotation.clamped(),
            flip_h: self.flip_h,
            flip_v: self.flip_v,
            opacity: PerMille::new(self.opacity.get().min(PerMille::ONE.get())),
        }
    }
}

impl Default for Transform {
    fn default() -> Transform {
        Transform::IDENTITY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_follow_tr_001() {
        let wild = Transform {
            position: CanvasPos::new(i32::MAX, -9_000_000),
            scale: Scale::from_basis_points(-5),
            rotation: Angle::from_degrees(5000),
            flip_h: true,
            flip_v: false,
            opacity: PerMille::new(4000),
        };
        assert!(!wild.is_valid());
        let c = wild.clamped();
        assert!(c.is_valid());
        assert_eq!(c.position, CanvasPos::new(4_000_000, -4_000_000));
        assert_eq!(c.scale, Scale::from_percent(1));
        assert_eq!(c.rotation, Angle::from_degrees(3600));
        assert_eq!(c.opacity, PerMille::ONE);
        assert!(c.flip_h);
        assert_eq!(Scale::from_percent(2000).clamped(), Scale::MAX);
        assert_eq!(Angle::from_degrees(-3700).clamped().degrees(), -3600.0);
    }

    #[test]
    fn pixel_conversions_round_trip() {
        let p = CanvasPos::from_px(312.0, -40.0, 1920, 1080);
        let (x, y) = p.to_px(1920, 1080);
        assert!((x - 312.0).abs() < 1e-3 && (y + 40.0).abs() < 1e-3);
        assert_eq!(
            CanvasPos::from_px(1e12, f64::NAN, 1920, 1080).x,
            CanvasPos::LIMIT
        );
        assert_eq!(CanvasPos::from_px(0.0, f64::NAN, 1920, 1080).y, 0);
        assert_eq!(CanvasPos::from_px(960.0, 0.0, 1920, 1080).x, 500_000);
    }
}
