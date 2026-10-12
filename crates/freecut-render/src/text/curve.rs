//! Curved text (TX-016): every glyph sits on an arc and is turned to its
//! tangent.
//!
//! The curve amount `c` (−1..=1) bends the widest line into an arc of
//! `|c| × 360°`, so its radius is `width / (|c| · 2π)`. Positive amounts
//! make an arch (the centre below the text, the line's ends drop), negative
//! ones a bowl (the centre above). Every line shares the centre; lines sit
//! on concentric arcs one line pitch apart. The point of the widest line's
//! middle stays where it is at rest, and positions along a line become arc
//! lengths, so alignment is measured along the arc.

use super::layout::Built;

/// Below this amount text is drawn straight.
const MIN_CURVE: f32 = 1e-3;

pub(super) struct Arc {
    /// The circles' shared centre, layout px.
    cx: f32,
    cy: f32,
    /// Arch (centre below) or bowl (centre above).
    arch: bool,
    /// The x (at rest) that maps to the top (arch) or bottom (bowl) of the
    /// circle.
    mid: f32,
    /// Radius and rest baseline of each line.
    lines: Vec<(f32, f32)>,
}

impl Arc {
    /// The arc for `curve`, or `None` for straight text (no curve, or no
    /// line with any width).
    pub fn new(built: &Built, curve: f32) -> Option<Arc> {
        if !curve.is_finite() || curve.abs() < MIN_CURVE {
            return None;
        }
        let curve = curve.clamp(-1.0, 1.0);
        let widest = built
            .lines
            .iter()
            .max_by(|a, b| a.width.total_cmp(&b.width))?;
        if widest.width <= 0.0 {
            return None;
        }
        let radius = widest.width / (curve.abs() * std::f32::consts::TAU);
        let arch = curve > 0.0;
        let b0 = widest.baseline;
        let cy = if arch { b0 + radius } else { b0 - radius };
        // A floor so lines past the centre don't collapse to a point.
        let floor = 1e-2 * radius.max(1.0);
        let lines = built
            .lines
            .iter()
            .map(|l| {
                let dy = l.baseline - b0;
                let r = if arch { radius - dy } else { radius + dy };
                (r.max(floor), l.baseline)
            })
            .collect();
        Some(Arc {
            cx: widest.left + widest.width / 2.0,
            cy,
            arch,
            mid: widest.left + widest.width / 2.0,
            lines,
        })
    }

    fn line(&self, line: u32) -> (f32, f32) {
        self.lines
            .get(line as usize)
            .or(self.lines.last())
            .copied()
            .unwrap_or((1.0, 0.0))
    }

    /// Where the rest point (`x`, line baseline + `dy`) of `line` lands.
    pub fn point(&self, line: u32, x: f32, dy: f32) -> (f32, f32) {
        let (radius, _) = self.line(line);
        let a = (x - self.mid) / radius;
        let (s, c) = a.sin_cos();
        if self.arch {
            let r = radius - dy;
            (self.cx + r * s, self.cy - r * c)
        } else {
            let r = radius + dy;
            (self.cx + r * s, self.cy + r * c)
        }
    }

    /// The pen position and clockwise rotation (radians) of a glyph whose
    /// rest pen is at `x` on `line` with `advance`: its advance centre sits
    /// on the arc and it is turned to the tangent there.
    pub fn place(&self, line: u32, x: f32, advance: f32) -> ((f32, f32), f32) {
        let (radius, _) = self.line(line);
        let a = (x + advance / 2.0 - self.mid) / radius;
        let theta = if self.arch { a } else { -a };
        let (bx, by) = self.point(line, x + advance / 2.0, 0.0);
        let (s, c) = theta.sin_cos();
        ((bx - advance / 2.0 * c, by - advance / 2.0 * s), theta)
    }

    /// The rest position of a curved point drawn for `line` (the inverse of
    /// [`Arc::point`]).
    pub fn rest(&self, line: u32, p: (f32, f32)) -> (f32, f32) {
        let (radius, baseline) = self.line(line);
        let (dx, dy) = (p.0 - self.cx, p.1 - self.cy);
        let r = dx.hypot(dy);
        if self.arch {
            let a = dx.atan2(-dy);
            (self.mid + a * radius, baseline + (radius - r))
        } else {
            let a = dx.atan2(dy);
            (self.mid + a * radius, baseline + (r - radius))
        }
    }

    /// The radius of `line`'s arc.
    pub fn radius(&self, line: u32) -> f32 {
        self.line(line).0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arc(arch: bool) -> Arc {
        Arc {
            cx: 50.0,
            cy: if arch { 110.0 } else { -90.0 },
            arch,
            mid: 50.0,
            lines: vec![(100.0, 10.0)],
        }
    }

    #[test]
    fn the_middle_stays_and_the_ends_drop_on_an_arch() {
        let a = arc(true);
        let (x, y) = a.point(0, 50.0, 0.0);
        assert!((x - 50.0).abs() < 1e-4 && (y - 10.0).abs() < 1e-4);
        let (_, y_end) = a.point(0, 120.0, 0.0);
        assert!(y_end > 10.0);
        let b = arc(false);
        let (_, y_bowl) = b.point(0, 120.0, 0.0);
        assert!(y_bowl < 10.0);
    }

    #[test]
    fn rest_inverts_point() {
        for arch in [true, false] {
            let a = arc(arch);
            for (x, dy) in [(0.0, 0.0), (80.0, 5.0), (130.0, -12.0)] {
                let p = a.point(0, x, dy);
                let (rx, ry) = a.rest(0, p);
                assert!((rx - x).abs() < 1e-3, "{arch} {x}: {rx}");
                assert!((ry - (10.0 + dy)).abs() < 1e-3, "{arch} {dy}: {ry}");
            }
        }
    }

    #[test]
    fn glyphs_turn_clockwise_right_of_an_arch_top() {
        let a = arc(true);
        let (_, theta) = a.place(0, 80.0, 10.0);
        assert!(theta > 0.0);
        let (_, theta) = a.place(0, 10.0, 10.0);
        assert!(theta < 0.0);
        // A bowl turns the other way.
        let (_, theta) = arc(false).place(0, 80.0, 10.0);
        assert!(theta < 0.0);
    }
}
