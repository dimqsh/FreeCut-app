//! Curves (CL-014): the master (Luma) curve and the Red, Green and Blue
//! curves, each a short list of points in fixed point.
//!
//! Coordinates are `u16` in `0..=10000` (CL-001's model notes). A curve
//! always has its two endpoints; [`Curve::new`] is the only way to build
//! one, so every [`Curve`] in a document is valid. The interpolation (a
//! monotone cubic) and the baked tables live in `freecut-render`.

/// One point of a curve: input `x` and output `y`, both `0..=10000`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CurvePoint {
    pub x: u16,
    pub y: u16,
}

impl CurvePoint {
    pub const fn new(x: u16, y: u16) -> CurvePoint {
        CurvePoint { x, y }
    }
}

/// Why a list of points is not a curve (CL-014's rules).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum CurveError {
    #[error("a curve needs its two endpoints")]
    TooFewPoints,
    #[error("a curve holds at most {max} points", max = Curve::MAX_POINTS)]
    TooManyPoints,
    #[error("point {index} lies outside 0–10000")]
    OutOfRange { index: usize },
    #[error("point {index} is less than 0.01 to the right of the point before it")]
    TooClose { index: usize },
    #[error("the left endpoint stays within x 0–0.25")]
    LeftEndpoint,
    #[error("the right endpoint stays within x 0.75–1")]
    RightEndpoint,
    #[error("the endpoints can't be removed")]
    Endpoint,
}

/// A curve (CL-014): 2 to 16 points with x strictly increasing by at least
/// [`Curve::MIN_GAP`], the left endpoint at x ≤ 0.25 and the right one at
/// x ≥ 0.75. Below the left endpoint and above the right one the output
/// holds the endpoint's level, like a levels black or white point.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Curve {
    points: Vec<CurvePoint>,
}

impl Default for Curve {
    fn default() -> Self {
        Curve::identity()
    }
}

impl Curve {
    /// 1.0 in curve coordinates.
    pub const ONE: u16 = 10_000;
    /// At most this many points, endpoints included.
    pub const MAX_POINTS: usize = 16;
    /// The least x step between neighbouring points (0.01).
    pub const MIN_GAP: u16 = 100;
    /// The left endpoint's x stays at or below this (0.25).
    pub const LEFT_MAX_X: u16 = 2_500;
    /// The right endpoint's x stays at or above this (0.75).
    pub const RIGHT_MIN_X: u16 = 7_500;

    /// The straight diagonal every curve starts as.
    pub fn identity() -> Curve {
        Curve {
            points: vec![CurvePoint::new(0, 0), CurvePoint::new(Self::ONE, Self::ONE)],
        }
    }

    /// `points` as a curve, if they follow CL-014's rules.
    pub fn new(points: Vec<CurvePoint>) -> Result<Curve, CurveError> {
        if points.len() < 2 {
            return Err(CurveError::TooFewPoints);
        }
        if points.len() > Self::MAX_POINTS {
            return Err(CurveError::TooManyPoints);
        }
        if let Some(index) = points
            .iter()
            .position(|p| p.x > Self::ONE || p.y > Self::ONE)
        {
            return Err(CurveError::OutOfRange { index });
        }
        if let Some(i) = points
            .windows(2)
            .position(|w| w[1].x < w[0].x.saturating_add(Self::MIN_GAP))
        {
            return Err(CurveError::TooClose { index: i + 1 });
        }
        if points[0].x > Self::LEFT_MAX_X {
            return Err(CurveError::LeftEndpoint);
        }
        if points[points.len() - 1].x < Self::RIGHT_MIN_X {
            return Err(CurveError::RightEndpoint);
        }
        Ok(Curve { points })
    }

    /// The points, left to right; the first and last are the endpoints.
    pub fn points(&self) -> &[CurvePoint] {
        &self.points
    }

    /// Whether this is the untouched diagonal.
    pub fn is_default(&self) -> bool {
        *self == Curve::identity()
    }

    /// Whether the curve leaves every value as it is: endpoints at the
    /// corners and every point on the diagonal (a monotone cubic through
    /// collinear points is that line).
    pub fn is_identity(&self) -> bool {
        let (first, last) = (self.points[0], self.points[self.points.len() - 1]);
        first == CurvePoint::new(0, 0)
            && last == CurvePoint::new(Self::ONE, Self::ONE)
            && self.points.iter().all(|p| p.x == p.y)
    }

    /// The curve with a point added at `p` (clicking the curve, CL-014):
    /// it must sit between the endpoints with room on both sides, and the
    /// curve must have fewer than 16 points.
    pub fn with_point(&self, p: CurvePoint) -> Result<Curve, CurveError> {
        let at = self.points.partition_point(|q| q.x < p.x);
        let mut points = self.points.clone();
        if at == 0 {
            return Err(CurveError::LeftEndpoint);
        }
        if at == points.len() {
            return Err(CurveError::RightEndpoint);
        }
        points.insert(at, p);
        Curve::new(points)
    }

    /// The curve without point `index`; the endpoints stay.
    pub fn without_point(&self, index: usize) -> Result<Curve, CurveError> {
        if index == 0 || index + 1 >= self.points.len() {
            return Err(CurveError::Endpoint);
        }
        let mut points = self.points.clone();
        points.remove(index);
        Curve::new(points)
    }

    /// The curve with point `index` moved toward `to` (a drag, CL-014): y is
    /// limited to 0–10000 and x to the room between its neighbours (±0.01)
    /// and the endpoint limits, so points never swap. An index past the
    /// end changes nothing.
    #[must_use]
    pub fn moved(&self, index: usize, to: CurvePoint) -> Curve {
        let last = self.points.len() - 1;
        if index > last {
            return self.clone();
        }
        let gap = Self::MIN_GAP;
        let lo = if index == 0 {
            0
        } else {
            self.points[index - 1].x + gap
        };
        let hi = if index == last {
            Self::ONE
        } else {
            self.points[index + 1].x - gap
        };
        let (lo, hi) = match (index == 0, index == last) {
            (true, _) => (lo, hi.min(Self::LEFT_MAX_X)),
            (_, true) => (lo.max(Self::RIGHT_MIN_X), hi),
            _ => (lo, hi),
        };
        let mut points = self.points.clone();
        points[index] = CurvePoint::new(to.x.clamp(lo, hi.max(lo)), to.y.min(Self::ONE));
        // The limits keep every rule, so this is the moved curve; the
        // fallback is never taken.
        Curve::new(points).unwrap_or_else(|_| self.clone())
    }
}

/// Which curve (CL-014's tabs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CurveChannel {
    Luma,
    Red,
    Green,
    Blue,
}

impl CurveChannel {
    pub const ALL: [CurveChannel; 4] = [
        CurveChannel::Luma,
        CurveChannel::Red,
        CurveChannel::Green,
        CurveChannel::Blue,
    ];

    /// The stable name in project files.
    pub const fn as_str(self) -> &'static str {
        match self {
            CurveChannel::Luma => "luma",
            CurveChannel::Red => "red",
            CurveChannel::Green => "green",
            CurveChannel::Blue => "blue",
        }
    }

    pub fn parse(s: &str) -> Option<CurveChannel> {
        CurveChannel::ALL.into_iter().find(|c| c.as_str() == s)
    }
}

/// The four curves (CL-014): Luma applies to R, G and B first, then each
/// channel's own curve.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Curves {
    pub luma: Curve,
    pub red: Curve,
    pub green: Curve,
    pub blue: Curve,
}

impl Curves {
    pub fn get(&self, channel: CurveChannel) -> &Curve {
        match channel {
            CurveChannel::Luma => &self.luma,
            CurveChannel::Red => &self.red,
            CurveChannel::Green => &self.green,
            CurveChannel::Blue => &self.blue,
        }
    }

    pub fn get_mut(&mut self, channel: CurveChannel) -> &mut Curve {
        match channel {
            CurveChannel::Luma => &mut self.luma,
            CurveChannel::Red => &mut self.red,
            CurveChannel::Green => &mut self.green,
            CurveChannel::Blue => &mut self.blue,
        }
    }

    /// Whether every curve leaves values as they are.
    pub fn is_identity(&self) -> bool {
        CurveChannel::ALL.iter().all(|c| self.get(*c).is_identity())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pts(list: &[(u16, u16)]) -> Vec<CurvePoint> {
        list.iter().map(|(x, y)| CurvePoint::new(*x, *y)).collect()
    }

    #[test]
    fn new_enforces_every_rule() {
        assert_eq!(Curve::new(pts(&[(0, 0)])), Err(CurveError::TooFewPoints));
        let many: Vec<(u16, u16)> = (0..17).map(|i| (i * 600, i * 600)).collect();
        assert_eq!(Curve::new(pts(&many)), Err(CurveError::TooManyPoints));
        assert_eq!(
            Curve::new(pts(&[(0, 0), (10_000, 10_001)])),
            Err(CurveError::OutOfRange { index: 1 })
        );
        assert_eq!(
            Curve::new(pts(&[(0, 0), (5_000, 1), (5_099, 2), (10_000, 3)])),
            Err(CurveError::TooClose { index: 2 })
        );
        assert_eq!(
            Curve::new(pts(&[(2_501, 0), (10_000, 1)])),
            Err(CurveError::LeftEndpoint)
        );
        assert_eq!(
            Curve::new(pts(&[(0, 0), (7_499, 1)])),
            Err(CurveError::RightEndpoint)
        );
        let ok = Curve::new(pts(&[(2_500, 0), (5_000, 5_000), (5_100, 0), (7_500, 1)]));
        assert!(ok.is_ok());
        assert!(Curve::identity().is_identity() && Curve::identity().is_default());
        let on_diagonal = Curve::new(pts(&[(0, 0), (4_000, 4_000), (10_000, 10_000)])).unwrap();
        assert!(on_diagonal.is_identity() && !on_diagonal.is_default());
    }

    #[test]
    fn a_seventeenth_point_is_refused_and_endpoints_stay() {
        let mut c = Curve::identity();
        for i in 1..15u16 {
            c = c.with_point(CurvePoint::new(i * 600, 0)).unwrap();
        }
        assert_eq!(c.points().len(), 16);
        assert_eq!(
            c.with_point(CurvePoint::new(9_500, 1)),
            Err(CurveError::TooManyPoints)
        );
        assert_eq!(c.without_point(0), Err(CurveError::Endpoint));
        assert_eq!(c.without_point(15), Err(CurveError::Endpoint));
        assert_eq!(c.without_point(3).unwrap().points().len(), 15);
        // Points outside the endpoints are not added.
        let c = Curve::new(pts(&[(1_000, 0), (10_000, 10_000)])).unwrap();
        assert_eq!(
            c.with_point(CurvePoint::new(500, 1)),
            Err(CurveError::LeftEndpoint)
        );
    }

    #[test]
    fn moves_are_clamped_between_the_neighbours() {
        let c = Curve::new(pts(&[(0, 0), (5_000, 5_000), (10_000, 10_000)])).unwrap();
        let m = c.moved(1, CurvePoint::new(9_999, 20_000));
        assert_eq!(m.points()[1], CurvePoint::new(9_900, 10_000));
        let m = c.moved(0, CurvePoint::new(9_000, 3));
        assert_eq!(m.points()[0], CurvePoint::new(2_500, 3));
        let m = c.moved(2, CurvePoint::new(0, 7));
        assert_eq!(m.points()[2], CurvePoint::new(7_500, 7));
        assert_eq!(c.moved(9, CurvePoint::new(1, 1)), c);
    }
}
