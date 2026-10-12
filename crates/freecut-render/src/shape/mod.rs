//! Flat vector shapes: boxes and bubbles around text (TX-013, TX-055) and
//! text decorations, as closed polygons filled into coverage masks with
//! `zeno`.
//!
//! Every [`Path`] is a set of closed polygons (curves are flattened when
//! the path is built). Filling uses the non-zero rule after every contour
//! is turned the same way round, so overlapping contours form their
//! **union**: overlapping per-line boxes are one shape and never darken
//! where they meet.

pub mod bubble;

use zeno::{Command, Mask, Vector};

/// A point in layout (or image) px, y down.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Point {
        Point { x, y }
    }

    pub fn distance(self, other: Point) -> f32 {
        (self.x - other.x).hypot(self.y - other.y)
    }

    fn lerp(self, other: Point, t: f32) -> Point {
        Point::new(
            self.x + (other.x - self.x) * t,
            self.y + (other.y - self.y) * t,
        )
    }
}

/// An axis-aligned rectangle, `x0 <= x1` and `y0 <= y1` when valid.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Rect {
    pub const fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect { x0, y0, x1, y1 }
    }

    pub fn width(&self) -> f32 {
        self.x1 - self.x0
    }

    pub fn height(&self) -> f32 {
        self.y1 - self.y0
    }

    pub fn center(&self) -> Point {
        Point::new((self.x0 + self.x1) / 2.0, (self.y0 + self.y1) / 2.0)
    }

    /// Grown by `dx` on the left and right and `dy` on the top and bottom.
    pub fn outset(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(self.x0 - dx, self.y0 - dy, self.x1 + dx, self.y1 + dy)
    }

    /// The smallest rectangle holding both.
    pub fn union(&self, other: &Rect) -> Rect {
        Rect::new(
            self.x0.min(other.x0),
            self.y0.min(other.y0),
            self.x1.max(other.x1),
            self.y1.max(other.y1),
        )
    }

    pub fn is_empty(&self) -> bool {
        // NaN-safe: an undefined rectangle is empty.
        !(self.x1 > self.x0 && self.y1 > self.y0)
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x0 && p.x <= self.x1 && p.y >= self.y0 && p.y <= self.y1
    }
}

/// Closed polygons (the last point joins the first).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Path {
    pub contours: Vec<Vec<Point>>,
}

/// Segments per full turn when flattening circles and arcs. Fine enough
/// that a 2000 px circle deviates by less than 0.1 px.
const TURN_SEGMENTS: f32 = 360.0;

impl Path {
    pub fn new() -> Path {
        Path::default()
    }

    pub fn is_empty(&self) -> bool {
        self.contours.iter().all(|c| c.len() < 3)
    }

    /// Adds a closed polygon, dropping repeated points; polygons with
    /// fewer than three points are left out.
    pub fn push(&mut self, mut contour: Vec<Point>) {
        tidy(&mut contour);
        if contour.len() >= 3 {
            self.contours.push(contour);
        }
    }

    /// Adds every contour of `other`.
    pub fn extend(&mut self, other: Path) {
        self.contours.extend(other.contours);
    }

    /// The bounding box of all points; `None` when there are none.
    pub fn bounds(&self) -> Option<Rect> {
        let mut points = self.contours.iter().flatten();
        let first = points.next()?;
        let mut r = Rect::new(first.x, first.y, first.x, first.y);
        for p in points {
            r = r.union(&Rect::new(p.x, p.y, p.x, p.y));
        }
        Some(r)
    }

    /// Every point mapped through `f`.
    pub fn map(&self, f: impl Fn(Point) -> Point) -> Path {
        Path {
            contours: self
                .contours
                .iter()
                .map(|c| c.iter().map(|&p| f(p)).collect())
                .collect(),
        }
    }

    /// A rectangle with corners rounded by `radius` (clamped to half the
    /// shorter side).
    pub fn rounded_rect(r: Rect, radius: f32) -> Path {
        let mut path = Path::new();
        path.push(rounded_rect(r, radius));
        path
    }
}

/// Drops points that repeat their predecessor (within 1e-4 px), the last
/// one included when it repeats the first.
pub(crate) fn tidy(contour: &mut Vec<Point>) {
    contour.dedup_by(|b, a| a.distance(*b) < 1e-4);
    while contour.len() > 1 && contour[0].distance(contour[contour.len() - 1]) < 1e-4 {
        contour.pop();
    }
}

/// The polygon of a rectangle with rounded corners, clockwise on screen
/// (y down), starting at the top edge.
pub(crate) fn rounded_rect(r: Rect, radius: f32) -> Vec<Point> {
    if r.is_empty() {
        return Vec::new();
    }
    let radius = finite_or(radius, 0.0).clamp(0.0, r.width().min(r.height()) / 2.0);
    if radius < 0.01 {
        return vec![
            Point::new(r.x0, r.y0),
            Point::new(r.x1, r.y0),
            Point::new(r.x1, r.y1),
            Point::new(r.x0, r.y1),
        ];
    }
    let mut pts = Vec::new();
    // Corner centres and the angle each corner's quarter arc starts at
    // (screen angles: 0 = right, +90° = down).
    let corners = [
        (Point::new(r.x1 - radius, r.y0 + radius), -90.0_f32),
        (Point::new(r.x1 - radius, r.y1 - radius), 0.0),
        (Point::new(r.x0 + radius, r.y1 - radius), 90.0),
        (Point::new(r.x0 + radius, r.y0 + radius), 180.0),
    ];
    for (c, start) in corners {
        arc_points(&mut pts, c, radius, radius, start, start + 90.0, true);
    }
    tidy(&mut pts);
    pts
}

/// Appends points of an elliptical arc from `a0` to `a1` degrees (screen
/// angles), both ends included when `inclusive`.
pub(crate) fn arc_points(
    out: &mut Vec<Point>,
    c: Point,
    rx: f32,
    ry: f32,
    a0: f32,
    a1: f32,
    inclusive: bool,
) {
    let sweep = a1 - a0;
    let steps = ((sweep.abs() / 360.0 * TURN_SEGMENTS).ceil() as usize).max(2);
    let end = if inclusive { steps + 1 } else { steps };
    for i in 0..end {
        let a = (a0 + sweep * i as f32 / steps as f32).to_radians();
        out.push(Point::new(c.x + rx * a.cos(), c.y + ry * a.sin()));
    }
}

/// Points of a quadratic Bézier from `p0` (excluded) to `p2` (included).
pub(crate) fn quad_points(out: &mut Vec<Point>, p0: Point, ctrl: Point, p2: Point) {
    let len = p0.distance(ctrl) + ctrl.distance(p2);
    let steps = ((len / 2.0).ceil() as usize).clamp(4, 64);
    for i in 1..=steps {
        let t = i as f32 / steps as f32;
        let a = p0.lerp(ctrl, t);
        let b = ctrl.lerp(p2, t);
        out.push(a.lerp(b, t));
    }
}

/// Twice the signed area: positive for clockwise polygons on screen
/// (y down).
pub(crate) fn signed_area2(c: &[Point]) -> f32 {
    let mut s = 0.0;
    for (i, p) in c.iter().enumerate() {
        let q = c[(i + 1) % c.len()];
        s += p.x * q.y - q.x * p.y;
    }
    s
}

pub(crate) fn finite_or(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

/// A coverage mask: one byte per pixel, rows top to bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Coverage {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

impl Coverage {
    pub fn new(width: usize, height: usize) -> Coverage {
        Coverage {
            width,
            height,
            data: vec![0; width * height],
        }
    }
}

/// Fills `path` (in image px) into `dst` as the union with what is
/// there (per-pixel maximum). Contours are oriented consistently first,
/// so overlapping contours never cancel or add up.
pub(crate) fn fill_union(dst: &mut Coverage, path: &Path) {
    let Some(b) = path.bounds() else {
        return;
    };
    let x0 = (b.x0.floor().max(0.0)) as i64;
    let y0 = (b.y0.floor().max(0.0)) as i64;
    let x1 = (b.x1.ceil().min(dst.width as f32)) as i64;
    let y1 = (b.y1.ceil().min(dst.height as f32)) as i64;
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let mut commands = Vec::new();
    for contour in &path.contours {
        if contour.len() < 3 {
            continue;
        }
        let clockwise = signed_area2(contour) >= 0.0;
        let mut push = |i: usize, first: bool| {
            let p = contour[i];
            let v = Vector::new(p.x - x0 as f32, p.y - y0 as f32);
            commands.push(if first {
                Command::MoveTo(v)
            } else {
                Command::LineTo(v)
            });
        };
        if clockwise {
            for i in 0..contour.len() {
                push(i, i == 0);
            }
        } else {
            for (n, i) in (0..contour.len()).rev().enumerate() {
                push(i, n == 0);
            }
        }
        commands.push(Command::Close);
    }
    if commands.is_empty() {
        return;
    }
    let mut local = vec![0u8; w * h];
    Mask::new(&commands[..])
        .size(w as u32, h as u32)
        .render_into(&mut local, None);
    for row in 0..h {
        let src = &local[row * w..(row + 1) * w];
        let start = (y0 as usize + row) * dst.width + x0 as usize;
        for (d, &s) in dst.data[start..start + w].iter_mut().zip(src) {
            *d = (*d).max(s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coverage_sum(c: &Coverage) -> u64 {
        c.data.iter().map(|&v| u64::from(v)).sum()
    }

    #[test]
    fn a_whole_pixel_rectangle_fills_exactly() {
        let mut c = Coverage::new(10, 10);
        fill_union(
            &mut c,
            &Path::rounded_rect(Rect::new(2.0, 3.0, 6.0, 5.0), 0.0),
        );
        for y in 0..10 {
            for x in 0..10 {
                let inside = (2..6).contains(&x) && (3..5).contains(&y);
                assert_eq!(c.data[y * 10 + x], if inside { 255 } else { 0 }, "{x},{y}");
            }
        }
    }

    #[test]
    fn overlapping_contours_form_a_union_whatever_their_direction() {
        let mut one = Path::rounded_rect(Rect::new(1.0, 1.0, 7.0, 5.0), 0.0);
        let mut reversed = rounded_rect(Rect::new(4.0, 3.0, 9.0, 9.0), 0.0);
        reversed.reverse();
        one.push(reversed);
        let mut c = Coverage::new(10, 10);
        fill_union(&mut c, &one);
        assert!(c.data.iter().all(|&v| v == 0 || v == 255));
        // 6×4 + 5×6 − overlap 3×2.
        assert_eq!(coverage_sum(&c), 255 * (24 + 30 - 6));
    }

    #[test]
    fn rounded_corners_cover_less_and_stay_inside() {
        let r = Rect::new(0.0, 0.0, 40.0, 20.0);
        let pill = rounded_rect(r, 10.0);
        assert!(pill.iter().all(|p| r.outset(1e-3, 1e-3).contains(*p)));
        let area = signed_area2(&pill) / 2.0;
        let exact = 20.0 * 20.0 + std::f32::consts::PI * 100.0;
        assert!((area - exact).abs() < 1.0, "{area} vs {exact}");
    }

    #[test]
    fn filling_is_clipped_to_the_mask() {
        let mut c = Coverage::new(4, 4);
        fill_union(
            &mut c,
            &Path::rounded_rect(Rect::new(-10.0, -10.0, 2.0, 50.0), 0.0),
        );
        assert_eq!(coverage_sum(&c), 255 * 8);
    }
}
