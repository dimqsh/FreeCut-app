//! Curves as tables (CL-014, render-compositor.md §17): a curve's points
//! through a Fritsch–Carlson monotone cubic, baked to 1024 `f32` entries
//! and read with linear interpolation.
//!
//! The table is built in `f64` (plain arithmetic, no transcendental
//! functions, so the bits are the same on every OS) and stored as `f32`.
//! Below the left endpoint and above the right one the curve holds the
//! endpoint's level.

use freecut_timeline::model::{Curve, CurveChannel, CurvePoint, Curves};

/// How many entries a curve table has (CL-014).
pub const CURVE_TABLE_SIZE: usize = 1024;

/// A curve baked for lookups.
#[derive(Debug, Clone, PartialEq)]
pub struct CurveTable {
    values: Box<[f32]>,
}

/// One coordinate of a point as `0..=1`.
fn unit(v: u16) -> f64 {
    f64::from(v) / f64::from(Curve::ONE)
}

/// Fritsch–Carlson tangents: the curve is monotone on every segment, so it
/// never overshoots or inverts between two points.
fn tangents(xs: &[f64], ys: &[f64]) -> Vec<f64> {
    let n = xs.len();
    let d: Vec<f64> = (0..n - 1)
        .map(|k| (ys[k + 1] - ys[k]) / (xs[k + 1] - xs[k]))
        .collect();
    let mut m = vec![0.0; n];
    m[0] = d[0];
    m[n - 1] = d[n - 2];
    for k in 1..n - 1 {
        m[k] = if d[k - 1] * d[k] > 0.0 {
            (d[k - 1] + d[k]) / 2.0
        } else {
            0.0
        };
    }
    for k in 0..n - 1 {
        if d[k] == 0.0 {
            m[k] = 0.0;
            m[k + 1] = 0.0;
            continue;
        }
        let a = m[k] / d[k];
        let b = m[k + 1] / d[k];
        // A tangent pointing against its segment (an endpoint next to a
        // turn) would leave the segment's range.
        if a < 0.0 {
            m[k] = 0.0;
        }
        if b < 0.0 {
            m[k + 1] = 0.0;
        }
        let s = a * a + b * b;
        if s > 9.0 {
            let t = 3.0 / s.sqrt();
            m[k] = t * a * d[k];
            m[k + 1] = t * b * d[k];
        }
    }
    m
}

/// The monotone cubic through `points` at `x`.
fn eval_curve(xs: &[f64], ys: &[f64], m: &[f64], x: f64) -> f64 {
    let n = xs.len();
    if x <= xs[0] {
        return ys[0];
    }
    if x >= xs[n - 1] {
        return ys[n - 1];
    }
    let k = xs.partition_point(|v| *v <= x).saturating_sub(1).min(n - 2);
    let h = xs[k + 1] - xs[k];
    let t = (x - xs[k]) / h;
    let (t2, t3) = (t * t, t * t * t);
    let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
    let h10 = t3 - 2.0 * t2 + t;
    let h01 = -2.0 * t3 + 3.0 * t2;
    let h11 = t3 - t2;
    h00 * ys[k] + h10 * h * m[k] + h01 * ys[k + 1] + h11 * h * m[k + 1]
}

impl CurveTable {
    pub fn new(curve: &Curve) -> CurveTable {
        CurveTable::from_points(curve.points())
    }

    fn from_points(points: &[CurvePoint]) -> CurveTable {
        let xs: Vec<f64> = points.iter().map(|p| unit(p.x)).collect();
        let ys: Vec<f64> = points.iter().map(|p| unit(p.y)).collect();
        let m = tangents(&xs, &ys);
        let last = (CURVE_TABLE_SIZE - 1) as f64;
        let values = (0..CURVE_TABLE_SIZE)
            .map(|i| eval_curve(&xs, &ys, &m, i as f64 / last) as f32)
            .collect();
        CurveTable { values }
    }

    /// The curve at `v`, clamped into 0–1 first (CL-014).
    #[inline]
    pub fn lookup(&self, v: f32) -> f32 {
        let p = v.clamp(0.0, 1.0) * (CURVE_TABLE_SIZE - 1) as f32;
        // In 0..=1023 after the clamp, so the cast is exact.
        let i = (p as usize).min(CURVE_TABLE_SIZE - 2);
        let f = p - i as f32;
        let (a, b) = (self.values[i], self.values[i + 1]);
        a + f * (b - a)
    }

    /// The table's entries, left to right.
    pub fn values(&self) -> &[f32] {
        &self.values
    }

    /// The steepest step of the table, as a slope (`|Δy| / Δx`): what
    /// decides the lattice size (README C52).
    pub fn max_slope(&self) -> f32 {
        let step = (CURVE_TABLE_SIZE - 1) as f32;
        self.values
            .windows(2)
            .map(|w| (w[1] - w[0]).abs() * step)
            .fold(0.0, f32::max)
    }
}

/// Stage 9 (CL-014): the Luma curve on R, G and B alike, then each
/// channel's own curve. Curves on the diagonal are skipped.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CurvesStage {
    luma: Option<CurveTable>,
    rgb: [Option<CurveTable>; 3],
}

impl CurvesStage {
    /// The stage for `curves`; `None` when every curve is the diagonal.
    pub(crate) fn new(curves: &Curves) -> Option<CurvesStage> {
        if curves.is_identity() {
            return None;
        }
        let table = |c: CurveChannel| {
            let curve = curves.get(c);
            (!curve.is_identity()).then(|| CurveTable::new(curve))
        };
        Some(CurvesStage {
            luma: table(CurveChannel::Luma),
            rgb: [
                table(CurveChannel::Red),
                table(CurveChannel::Green),
                table(CurveChannel::Blue),
            ],
        })
    }

    pub(crate) fn apply(&self, c: [f32; 3]) -> [f32; 3] {
        let mut out = c;
        for (i, v) in out.iter_mut().enumerate() {
            if let Some(t) = &self.luma {
                *v = t.lookup(*v);
            }
            if let Some(t) = &self.rgb[i] {
                *v = t.lookup(*v);
            }
        }
        out
    }

    /// The steepest slope a channel goes through: the Luma table's times
    /// the channel's own, since one follows the other.
    pub(crate) fn max_slope(&self) -> f32 {
        let luma = self.luma.as_ref().map_or(1.0, CurveTable::max_slope);
        self.rgb
            .iter()
            .map(|t| luma * t.as_ref().map_or(1.0, CurveTable::max_slope))
            .fold(0.0, f32::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve(list: &[(u16, u16)]) -> Curve {
        Curve::new(list.iter().map(|(x, y)| CurvePoint::new(*x, *y)).collect()).unwrap()
    }

    #[test]
    fn the_diagonal_is_exact_and_ends_clip() {
        let t = CurveTable::new(&Curve::identity());
        for i in 0..CURVE_TABLE_SIZE {
            let x = i as f32 / 1023.0;
            assert!((t.values()[i] - x).abs() < 1e-6);
        }
        assert_eq!(t.lookup(-3.0), 0.0);
        assert_eq!(t.lookup(7.0), 1.0);
        // Moving the endpoints in clips like a levels control.
        let levels = CurveTable::new(&curve(&[(2_000, 0), (8_000, 10_000)]));
        assert_eq!(levels.lookup(0.1), 0.0);
        assert_eq!(levels.lookup(0.9), 1.0);
        assert!((levels.lookup(0.5) - 0.5).abs() < 1e-3);
        assert!((levels.max_slope() - 1.0 / 0.6).abs() < 0.01);
    }

    #[test]
    fn no_overshoot_between_points() {
        // Steps and spikes: a monotone cubic stays within each segment's
        // two levels, so a sharp rise never rings past them.
        for list in [
            &[(0, 0), (4_000, 0), (4_200, 10_000), (10_000, 10_000)][..],
            &[(0, 10_000), (5_000, 0), (5_100, 10_000), (10_000, 0)],
            &[
                (0, 0),
                (1_000, 9_000),
                (2_000, 1_000),
                (3_000, 9_500),
                (10_000, 2_000),
            ],
        ] {
            let c = curve(list);
            let t = CurveTable::new(&c);
            let pts = c.points();
            for (i, v) in t.values().iter().enumerate() {
                let x = (i as f64 / 1023.0 * 10_000.0) as u16;
                let k = pts.partition_point(|p| p.x <= x).clamp(1, pts.len() - 1);
                let (a, b) = (pts[k - 1], pts[k]);
                let lo = f32::from(a.y.min(b.y)) / 10_000.0;
                let hi = f32::from(a.y.max(b.y)) / 10_000.0;
                assert!(*v >= lo - 1e-6 && *v <= hi + 1e-6, "{list:?} at {x}: {v}");
            }
        }
    }
}
