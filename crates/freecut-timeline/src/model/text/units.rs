//! Fixed-point scalars of the text style that TR's units don't cover (TX
//! conventions): signed ratios and angles in tenths of a degree.

use crate::model::PerMille;

/// A signed ratio in thousandths (1000 = 100 %), for values that go below
/// zero: letter spacing (TX-009), curve amount (TX-016), tail bend
/// (TX-055). [`PerMille`] is unsigned.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SignedPerMille(i16);

impl SignedPerMille {
    pub const ZERO: SignedPerMille = SignedPerMille(0);

    pub const fn new(per_mille: i16) -> SignedPerMille {
        SignedPerMille(per_mille)
    }

    pub const fn get(self) -> i16 {
        self.0
    }

    /// The ratio as a factor (1000 → 1.0), for the render and UI edge.
    pub fn as_f64(self) -> f64 {
        f64::from(self.0) / 1000.0
    }

    pub(crate) fn clamp(self, min: i16, max: i16) -> SignedPerMille {
        SignedPerMille(self.0.clamp(min, max))
    }
}

/// An angle in tenths of a degree, `0..3600`, measured clockwise from the
/// right (TX-011): 900 points down. Always in range: construction wraps.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeciDegrees(u16);

impl DeciDegrees {
    pub const ZERO: DeciDegrees = DeciDegrees(0);
    /// 45°, down and to the right: TX-011's and TX-012's default.
    pub const DOWN_RIGHT: DeciDegrees = DeciDegrees(450);
    /// 90°: top to bottom, TX-014's default linear angle.
    pub const DOWN: DeciDegrees = DeciDegrees(900);
    const TURN: i32 = 3600;

    /// `tenths` of a degree, wrapped into one turn (−10° is 350°).
    pub const fn new(tenths: i32) -> DeciDegrees {
        // `rem_euclid` puts the value in 0..3600, which fits a u16.
        DeciDegrees(tenths.rem_euclid(Self::TURN) as u16)
    }

    /// Whole degrees, wrapped.
    pub const fn from_degrees(degrees: i32) -> DeciDegrees {
        DeciDegrees::new(degrees.wrapping_rem(360).wrapping_mul(10))
    }

    pub const fn tenths(self) -> u16 {
        self.0
    }

    /// Degrees, for the render and UI edge.
    pub fn degrees(self) -> f64 {
        f64::from(self.0) / 10.0
    }
}

/// `p` clamped to `min..=max` per-mille.
pub(crate) fn clamp_pm(p: PerMille, min: u16, max: u16) -> PerMille {
    PerMille::new(p.get().clamp(min, max))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angles_wrap_into_one_turn() {
        assert_eq!(DeciDegrees::new(3600).tenths(), 0);
        assert_eq!(DeciDegrees::new(-100).tenths(), 3500);
        assert_eq!(DeciDegrees::new(i32::MIN).tenths(), 2752);
        assert_eq!(DeciDegrees::from_degrees(-45).tenths(), 3150);
        assert_eq!(DeciDegrees::from_degrees(725), DeciDegrees::new(50));
        assert_eq!(DeciDegrees::DOWN_RIGHT.degrees(), 45.0);
    }

    #[test]
    fn signed_ratios_clamp() {
        assert_eq!(SignedPerMille::new(-900).clamp(-200, 1000).get(), -200);
        assert_eq!(SignedPerMille::new(-150).as_f64(), -0.15);
    }
}
