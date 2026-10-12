//! Bubble and tag shapes around text (TX-055), drawn from parameters.
//!
//! The eight kinds are FreeCut's own parametric shapes. Every body outline
//! is star-shaped around the body centre and listed clockwise on screen in
//! order of angle, which is what makes the tail simple to attach: the
//! points within the tail's angular span are replaced by the tail, so body
//! and tail are one closed outline (filled once, bordered once, no seam).
//!
//! The tail's base faces the tip: its centre is the point of the body
//! outline nearest the tip, and its ends lie half the base width either
//! side of it, measured along the outline, so the base is the TX-055 width
//! wherever the tip is (a Shout's base widens to the valleys around it). A
//! tip inside the body hides the tail. The Thought kind draws a trail of
//! shrinking circles towards the tip instead of a tail.

use super::{Path, Point, Rect, arc_points, finite_or, quad_points, rounded_rect, tidy};

/// Which end of an [`BubbleKind::ArrowTag`] is pointed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    #[default]
    Right,
}

/// The shape family (TX-055).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum BubbleKind {
    /// A rounded rectangle; corner radius from the roundness.
    #[default]
    Rounded,
    /// An ellipse.
    Oval,
    /// A cloud of lobes with a trail of circles.
    Thought,
    /// A burst of spikes.
    Shout,
    /// A sharp-cornered rectangle.
    Square,
    /// A pill.
    Tag,
    /// A label with one pointed end.
    ArrowTag { point: Side },
    /// A rectangle with notched corners.
    Ticket,
}

impl BubbleKind {
    /// Whether the kind can have a tail ([`BubbleSpec::tail`]); Thought
    /// draws its trail instead.
    pub fn has_tail(self) -> bool {
        matches!(
            self,
            BubbleKind::Rounded | BubbleKind::Oval | BubbleKind::Square | BubbleKind::Shout
        )
    }

    /// Whether the body is an ellipse grown to hold the text box's corners.
    fn is_elliptic(self) -> bool {
        matches!(
            self,
            BubbleKind::Oval | BubbleKind::Thought | BubbleKind::Shout
        )
    }
}

/// A bubble's tail, or a Thought bubble's trail.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tail {
    /// The tip, relative to the body centre (the text box centre), in the
    /// same units as the body rectangle.
    pub tip: Point,
    /// Width at the base: a fraction of the body's shorter side
    /// (0.10..=0.60, TX-055 default 0.24).
    pub base: f32,
    /// −1..=1: 0 is a straight wedge; other values curve it to one side.
    pub bend: f32,
}

impl Tail {
    /// TX-055's default tail for a text box of `text_size` (w, h): the tip
    /// a quarter of the box width left of centre and 1.2 em below the
    /// bottom edge.
    pub fn default_for(text_size: (f32, f32), em: f32) -> Tail {
        Tail {
            tip: Point::new(-text_size.0 / 4.0, text_size.1 / 2.0 + 1.2 * em),
            base: 0.24,
            bend: 0.2,
        }
    }
}

/// The parameters of a bubble outline (TX-055). Fractions are of the
/// body's shorter side unless noted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BubbleSpec {
    pub kind: BubbleKind,
    /// The tail (Rounded, Oval, Square, Shout) or the trail (Thought);
    /// ignored by the other kinds.
    pub tail: Option<Tail>,
    /// Shout: spikes, 8..=32.
    pub spikes: u8,
    /// Shout: spike depth, 0.05..=0.40.
    pub spike_depth: f32,
    /// Shout: irregularity, 0..=1.
    pub jitter: f32,
    /// Shout: seed of the irregularity; the same seed gives the same burst.
    pub seed: u32,
    /// Thought: lobe size, 0.10..=0.40.
    pub lobe: f32,
    /// Thought: circles in the trail, 2..=4.
    pub trail: u8,
}

impl Default for BubbleSpec {
    /// TX-055's defaults, without a tail.
    fn default() -> BubbleSpec {
        BubbleSpec {
            kind: BubbleKind::Rounded,
            tail: None,
            spikes: 14,
            spike_depth: 0.18,
            jitter: 0.3,
            seed: 0,
            lobe: 0.22,
            trail: 3,
        }
    }
}

/// The body rectangle for `kind` around a text box `text`, with padding
/// `pad` (x, y) and a minimum body of 2 em × 1.5 em (TX-055). Elliptic
/// bodies (Oval, Thought, Shout) grow until the text box's corners are
/// inside the ellipse.
pub fn bubble_body(kind: BubbleKind, text: Rect, pad: (f32, f32), em: f32) -> Rect {
    let c = text.center();
    let (pad_x, pad_y) = (
        finite_or(pad.0, 0.0).max(0.0),
        finite_or(pad.1, 0.0).max(0.0),
    );
    let (mut hw, mut hh) = (text.width().max(0.0) / 2.0, text.height().max(0.0) / 2.0);
    if kind.is_elliptic() {
        hw *= std::f32::consts::SQRT_2;
        hh *= std::f32::consts::SQRT_2;
    }
    let em = finite_or(em, 0.0).max(0.0);
    hw = (hw + pad_x).max(em);
    hh = (hh + pad_y).max(0.75 * em);
    Rect::new(c.x - hw, c.y - hh, c.x + hw, c.y + hh)
}

/// The closed outline of a bubble with body `body` and corner `roundness`
/// (0..=1; 1 makes the corner radius half the shorter side). Body and
/// tail are one contour; a Thought bubble's trail circles are extra
/// contours. Empty for an empty body.
pub fn bubble_path(spec: &BubbleSpec, body: Rect, roundness: f32) -> Path {
    let mut path = Path::new();
    if body.is_empty() {
        return path;
    }
    let outline = body_outline(spec, body, roundness);
    let c = body.center();
    match (spec.kind, spec.tail) {
        (kind, Some(tail)) if kind.has_tail() => {
            let tip = Point::new(c.x + tail.tip.x, c.y + tail.tip.y);
            match attach_tail(spec, &outline, body, tip, &tail) {
                Some((contour, _)) => path.push(contour),
                None => path.push(outline),
            }
        }
        (BubbleKind::Thought, Some(tail)) => {
            let tip = Point::new(c.x + tail.tip.x, c.y + tail.tip.y);
            path.push(outline.clone());
            for circle in trail(spec, &outline, body, tip) {
                path.push(circle);
            }
        }
        _ => path.push(outline),
    }
    path
}

/// The two ends of the tail's base on the body outline (where the tail
/// leaves and rejoins it), or `None` when there is no tail: a kind without
/// one, no tail set, or the tip inside the body.
pub fn tail_base(spec: &BubbleSpec, body: Rect, roundness: f32) -> Option<(Point, Point)> {
    let tail = spec.tail?;
    if !spec.kind.has_tail() || body.is_empty() {
        return None;
    }
    let outline = body_outline(spec, body, roundness);
    let c = body.center();
    let tip = Point::new(c.x + tail.tip.x, c.y + tail.tip.y);
    attach_tail(spec, &outline, body, tip, &tail).map(|(_, base)| base)
}

/// The tail-less body outline: clockwise on screen, star-shaped around the
/// body centre, points in order of angle.
fn body_outline(spec: &BubbleSpec, body: Rect, roundness: f32) -> Vec<Point> {
    let short = body.width().min(body.height());
    let roundness = finite_or(roundness, 0.0).clamp(0.0, 1.0);
    let radius = roundness * short / 2.0;
    let c = body.center();
    let (rx, ry) = (body.width() / 2.0, body.height() / 2.0);
    let mut outline = match spec.kind {
        BubbleKind::Rounded => rounded_rect(body, radius),
        BubbleKind::Square => rounded_rect(body, 0.0),
        BubbleKind::Tag => rounded_rect(body, short / 2.0),
        BubbleKind::Oval => {
            let mut pts = Vec::new();
            arc_points(&mut pts, c, rx, ry, -180.0, 180.0, false);
            pts
        }
        BubbleKind::Thought => thought_outline(spec, body),
        BubbleKind::Shout => burst(spec, body),
        BubbleKind::ArrowTag { point } => arrow_tag(body, radius, point),
        BubbleKind::Ticket => ticket(body, radius),
    };
    tidy(&mut outline);
    outline
}

fn arrow_tag(body: Rect, radius: f32, point: Side) -> Vec<Point> {
    let h = body.height();
    let r = radius.min(h / 2.0);
    let cy = body.center().y;
    // Pointing right; mirrored below for a left point.
    let mut pts = Vec::new();
    arc_points(
        &mut pts,
        Point::new(body.x0 + r, body.y0 + r),
        r,
        r,
        180.0,
        270.0,
        true,
    );
    pts.push(Point::new(body.x1, body.y0));
    pts.push(Point::new(body.x1 + h / 2.0, cy));
    pts.push(Point::new(body.x1, body.y1));
    arc_points(
        &mut pts,
        Point::new(body.x0 + r, body.y1 - r),
        r,
        r,
        90.0,
        180.0,
        true,
    );
    tidy(&mut pts);
    match point {
        Side::Right => pts,
        Side::Left => {
            // Mirror around the body's vertical centre line, keeping the
            // pointed end the same distance outside the body.
            let mirror = body.x0 + body.x1;
            let mut m: Vec<Point> = pts.iter().map(|p| Point::new(mirror - p.x, p.y)).collect();
            m.reverse();
            m
        }
    }
}

fn ticket(body: Rect, radius: f32) -> Vec<Point> {
    let n = radius.min(body.width().min(body.height()) / 2.0);
    if n < 0.01 {
        return rounded_rect(body, 0.0);
    }
    let mut pts = Vec::new();
    // Concave quarter circles centred on each corner, clockwise.
    let corners = [
        (Point::new(body.x1, body.y0), 180.0_f32, 90.0_f32),
        (Point::new(body.x1, body.y1), 270.0, 180.0),
        (Point::new(body.x0, body.y1), 0.0, -90.0),
        (Point::new(body.x0, body.y0), 90.0, 0.0),
    ];
    for (c, a0, a1) in corners {
        arc_points(&mut pts, c, n, n, a0, a1, true);
    }
    tidy(&mut pts);
    pts
}

/// Deterministic 0..1 values from a seed (SplitMix64).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        // 24 random bits: exactly representable in f32.
        (z >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// The point of the body ellipse at screen angle `a` (radians) from its
/// centre, and the unit vector of that direction.
fn ellipse_ray(c: Point, rx: f32, ry: f32, a: f32) -> Point {
    let (s, co) = a.sin_cos();
    // Distance along the ray to the ellipse.
    let r = 1.0 / ((co / rx).powi(2) + (s / ry).powi(2)).sqrt();
    Point::new(c.x + r * co, c.y + r * s)
}

/// A burst: valleys on the body ellipse, spike tips beyond it.
fn burst(spec: &BubbleSpec, body: Rect) -> Vec<Point> {
    let c = body.center();
    let (rx, ry) = (body.width() / 2.0, body.height() / 2.0);
    let short = body.width().min(body.height());
    let n = usize::from(spec.spikes.clamp(8, 32));
    let depth = finite_or(spec.spike_depth, 0.18).clamp(0.05, 0.4) * short;
    let jitter = finite_or(spec.jitter, 0.0).clamp(0.0, 1.0);
    let mut rng = Rng(u64::from(spec.seed));
    let step = std::f32::consts::TAU / (2 * n) as f32;
    let mut pts = Vec::with_capacity(2 * n);
    for i in 0..2 * n {
        // Angles move by at most 0.3 of a step, so they stay increasing
        // and the outline stays simple.
        let a = -std::f32::consts::PI
            + step * (i as f32 + 0.5)
            + jitter * (rng.next() - 0.5) * 0.6 * step;
        let on = ellipse_ray(c, rx, ry, a);
        if i % 2 == 0 {
            let d = depth * (1.0 + jitter * (rng.next() - 0.5) * 1.2);
            let (s, co) = a.sin_cos();
            pts.push(Point::new(on.x + d * co, on.y + d * s));
        } else {
            pts.push(on);
        }
    }
    pts
}

fn thought_outline(spec: &BubbleSpec, body: Rect) -> Vec<Point> {
    let c = body.center();
    let (rx, ry) = (body.width() / 2.0, body.height() / 2.0);
    let short = body.width().min(body.height());
    let lobe = finite_or(spec.lobe, 0.22).clamp(0.1, 0.4) * short;
    // Ramanujan's perimeter approximation.
    let h = ((rx - ry) / (rx + ry)).powi(2);
    let perimeter =
        std::f32::consts::PI * (rx + ry) * (1.0 + 3.0 * h / (10.0 + (4.0 - 3.0 * h).sqrt()));
    let n = ((perimeter / lobe).round() as usize).clamp(6, 48);
    let anchors: Vec<Point> = (0..n)
        .map(|i| {
            let a = -std::f32::consts::PI + std::f32::consts::TAU * i as f32 / n as f32;
            ellipse_ray(c, rx, ry, a)
        })
        .collect();
    let mut pts = Vec::new();
    for i in 0..n {
        let p = anchors[i];
        let q = anchors[(i + 1) % n];
        let chord = p.distance(q);
        if chord < 1e-3 {
            continue;
        }
        // A circular arc bulging outwards; the outward normal of a
        // clockwise edge (y down) is (dy, −dx).
        let (dx, dy) = ((q.x - p.x) / chord, (q.y - p.y) / chord);
        let normal = Point::new(dy, -dx);
        let sagitta = (0.35 * chord).min(0.5 * lobe);
        let radius = (chord * chord / 4.0 + sagitta * sagitta) / (2.0 * sagitta);
        let m = Point::new((p.x + q.x) / 2.0, (p.y + q.y) / 2.0);
        let centre = Point::new(
            m.x + normal.x * (sagitta - radius),
            m.y + normal.y * (sagitta - radius),
        );
        let a0 = (p.y - centre.y).atan2(p.x - centre.x).to_degrees();
        let mut a1 = (q.y - centre.y).atan2(q.x - centre.x).to_degrees();
        while a1 <= a0 {
            a1 += 360.0;
        }
        arc_points(&mut pts, centre, radius, radius, a0, a1, false);
    }
    pts
}

/// The angle (radians, −π..π) of `p` seen from `c`.
fn angle_of(c: Point, p: Point) -> f32 {
    (p.y - c.y).atan2(p.x - c.x)
}

/// `a − b` wrapped into −π..π.
fn angle_diff(a: f32, b: f32) -> f32 {
    let mut d = a - b;
    while d > std::f32::consts::PI {
        d -= std::f32::consts::TAU;
    }
    while d < -std::f32::consts::PI {
        d += std::f32::consts::TAU;
    }
    d
}

/// Where the ray from `c` at angle `a` leaves the star-shaped polygon
/// `poly`: the farthest crossing.
fn ray_exit(poly: &[Point], c: Point, a: f32) -> Option<Point> {
    let (s, co) = a.sin_cos();
    let mut best: Option<f32> = None;
    for i in 0..poly.len() {
        let p = poly[i];
        let q = poly[(i + 1) % poly.len()];
        let (ex, ey) = (q.x - p.x, q.y - p.y);
        let denom = co * ey - s * ex;
        if denom.abs() < 1e-12 {
            continue;
        }
        let (wx, wy) = (p.x - c.x, p.y - c.y);
        let t = (wx * ey - wy * ex) / denom;
        let u = (wx * s - wy * co) / denom;
        if t > 0.0 && (-1e-6..=1.0 + 1e-6).contains(&u) {
            best = Some(best.map_or(t, |b: f32| b.max(t)));
        }
    }
    best.map(|t| Point::new(c.x + t * co, c.y + t * s))
}

/// Whether `p` is inside the polygon (even-odd crossing test).
fn contains(poly: &[Point], p: Point) -> bool {
    let mut inside = false;
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        if (a.y > p.y) != (b.y > p.y) {
            let x = a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x);
            if p.x < x {
                inside = !inside;
            }
        }
    }
    inside
}

/// The arc-length position along the closed polygon `poly` (from
/// `poly[0]`) of its point nearest `p`, and the polygon's perimeter.
fn nearest_on(poly: &[Point], p: Point) -> (f32, f32) {
    let n = poly.len();
    let (mut at, mut best, mut total) = (0.0, f32::INFINITY, 0.0);
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        let len = a.distance(b);
        let t = if len > 0.0 {
            (((p.x - a.x) * (b.x - a.x) + (p.y - a.y) * (b.y - a.y)) / (len * len)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let d = p.distance(a.lerp(b, t));
        if d < best {
            best = d;
            at = total + t * len;
        }
        total += len;
    }
    (at, total)
}

/// The point at arc-length position `s` (wrapped) along the closed polygon
/// `poly` of perimeter `total`.
fn point_along(poly: &[Point], s: f32, total: f32) -> Point {
    let n = poly.len();
    let mut s = s.rem_euclid(total.max(f32::MIN_POSITIVE));
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        let len = a.distance(b);
        if s <= len && len > 0.0 {
            return a.lerp(b, s / len);
        }
        s -= len;
    }
    poly[0]
}

/// The tail base's ends on the closed polygon `guide`: points either side
/// of arc position `at`, equally far along the outline, whose chord is
/// `width` (the arc is widened where the outline curves).
fn base_ends(guide: &[Point], at: f32, perimeter: f32, width: f32) -> (Point, Point) {
    let ends = |d: f32| {
        (
            point_along(guide, at - d, perimeter),
            point_along(guide, at + d, perimeter),
        )
    };
    let most = 0.45 * perimeter;
    let mut d = (width / 2.0).min(most);
    let mut pair = ends(d);
    for _ in 0..8 {
        let chord = pair.0.distance(pair.1);
        if chord >= 0.999 * width || d >= most {
            break;
        }
        d = (d * width / chord.max(1e-3)).min(most);
        pair = ends(d);
    }
    pair
}

/// The outline with the tail spliced in, and the base's two ends; `None`
/// when the tip is inside the body.
fn attach_tail(
    spec: &BubbleSpec,
    outline: &[Point],
    body: Rect,
    tip: Point,
    tail: &Tail,
) -> Option<(Vec<Point>, (Point, Point))> {
    if outline.len() < 3 || !tip.x.is_finite() || !tip.y.is_finite() || contains(outline, tip) {
        return None;
    }
    let c = body.center();
    let (rx, ry) = (body.width() / 2.0, body.height() / 2.0);
    let short = body.width().min(body.height());
    let width = finite_or(tail.base, 0.24).clamp(0.1, 0.6) * short;
    let (at, perimeter) = nearest_on(outline, tip);
    let q = point_along(outline, at, perimeter);
    // Outlines run clockwise, in order of angle: `a` before the base
    // centre, `b` after it.
    let (mut a, mut b) = base_ends(outline, at, perimeter, width);
    let (mut from, mut to) = (angle_of(c, a), angle_of(c, b));
    if spec.kind == BubbleKind::Shout {
        // Move each end to its nearest valley, so the neighbouring spikes
        // point away from the tail; at least the gap between the two
        // valleys around the base centre.
        let valleys: Vec<f32> = outline
            .iter()
            .filter(|p| {
                p.distance(c) <= ellipse_ray(c, rx, ry, angle_of(c, **p)).distance(c) + 1e-3
            })
            .map(|p| angle_of(c, *p))
            .collect();
        let nearest = |angle: f32, pick: &dyn Fn(f32) -> bool| {
            valleys
                .iter()
                .map(|&v| angle_diff(v, angle))
                .filter(|&d| pick(d))
                .min_by(|x, y| x.abs().total_cmp(&y.abs()))
                .map(|d| angle + d)
        };
        let centre = angle_of(c, q);
        // Valleys on either side of the base centre, apart.
        let around = |f: Option<f32>, t: Option<f32>| match (f, t) {
            (Some(f), Some(t))
                if angle_diff(t, f) > 1e-3
                    && angle_diff(centre, f) >= 0.0
                    && angle_diff(t, centre) >= 0.0 =>
            {
                Some((f, t))
            }
            _ => None,
        };
        let snapped = around(nearest(from, &|_| true), nearest(to, &|_| true)).or_else(|| {
            around(
                nearest(centre, &|d| d <= 0.0),
                nearest(centre, &|d| d >= 0.0),
            )
        });
        // Where the valleys are further apart than the widest base (the
        // ends of a long body), the base keeps its own width on the spike.
        if let Some((f, t)) = snapped
            && let (Some(fa), Some(tb)) = (
                ray_exit(outline, c, f - 1e-4),
                ray_exit(outline, c, t + 1e-4),
            )
            && fa.distance(tb) <= 0.6 * short
        {
            (from, to) = (f - 1e-4, t + 1e-4);
            (a, b) = (fa, tb);
        }
    }
    let turn = angle_diff(to, from);
    if turn <= 0.0 {
        return None;
    }
    // The outline points within the base's angular range give way to the
    // tail.
    let span = turn / 2.0;
    let axis = from + span;
    // The tail starts and ends on the body; keep the outline outside the
    // span, from just after `b` round to just before `a`.
    let n = outline.len();
    let first = (0..n).find(|&i| {
        let d = angle_diff(angle_of(c, outline[i]), axis);
        d > span && angle_diff(angle_of(c, outline[(i + n - 1) % n]), axis) <= span
    })?;
    let mut contour = vec![b];
    for k in 0..n {
        let p = outline[(first + k) % n];
        if angle_diff(angle_of(c, p), axis).abs() <= span {
            break;
        }
        contour.push(p);
    }
    contour.push(a);
    // Both edges bend the same way, perpendicular to the base→tip axis.
    let length = q.distance(tip);
    let (ux, uy) = (
        (tip.x - q.x) / length.max(1e-6),
        (tip.y - q.y) / length.max(1e-6),
    );
    let bend = finite_or(tail.bend, 0.0).clamp(-1.0, 1.0) * 0.2 * length;
    let (nx, ny) = (-uy * bend, ux * bend);
    let ctrl = |p: Point| Point::new((p.x + tip.x) / 2.0 + nx, (p.y + tip.y) / 2.0 + ny);
    quad_points(&mut contour, a, ctrl(a), tip);
    let before_b = contour.len();
    quad_points(&mut contour, tip, ctrl(b), b);
    // `b` already starts the contour.
    if contour.len() > before_b {
        contour.pop();
    }
    tidy(&mut contour);
    Some((contour, (a, b)))
}

/// The Thought trail: circles from the body edge to the tip, shrinking
/// towards it. None when the tip is inside the body.
fn trail(spec: &BubbleSpec, outline: &[Point], body: Rect, tip: Point) -> Vec<Vec<Point>> {
    if outline.len() < 3 || !tip.x.is_finite() || !tip.y.is_finite() || contains(outline, tip) {
        return Vec::new();
    }
    let c = body.center();
    let (rx, ry) = (body.width() / 2.0, body.height() / 2.0);
    let start = ellipse_ray(c, rx, ry, angle_of(c, tip));
    let count = usize::from(spec.trail.clamp(2, 4));
    let gap = start.distance(tip) / count as f32;
    let short = body.width().min(body.height());
    let lobe = finite_or(spec.lobe, 0.22).clamp(0.1, 0.4) * short;
    let biggest = (0.35 * lobe).min(0.4 * gap);
    if biggest < 0.05 {
        return Vec::new();
    }
    (0..count)
        .map(|i| {
            let t = (i + 1) as f32 / count as f32;
            let centre = Point::new(
                start.x + (tip.x - start.x) * t,
                start.y + (tip.y - start.y) * t,
            );
            let r = biggest * (1.0 - 0.5 * i as f32 / (count - 1) as f32);
            let mut pts = Vec::new();
            arc_points(&mut pts, centre, r, r, -180.0, 180.0, false);
            pts
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic_and_in_range() {
        let mut a = Rng(7);
        let mut b = Rng(7);
        for _ in 0..1000 {
            let v = a.next();
            assert_eq!(v, b.next());
            assert!((0.0..1.0).contains(&v));
        }
    }

    #[test]
    fn angle_diff_wraps() {
        assert!((angle_diff(3.0, -3.0) - (6.0 - std::f32::consts::TAU)).abs() < 1e-5);
        assert!((angle_diff(0.5, 0.25) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn rays_leave_a_square_on_its_edge() {
        let sq = rounded_rect(Rect::new(-1.0, -1.0, 1.0, 1.0), 0.0);
        let p = ray_exit(&sq, Point::new(0.0, 0.0), 0.0).unwrap();
        assert!((p.x - 1.0).abs() < 1e-5 && p.y.abs() < 1e-5);
        let p = ray_exit(&sq, Point::new(0.0, 0.0), std::f32::consts::FRAC_PI_2).unwrap();
        assert!((p.y - 1.0).abs() < 1e-5);
        assert!(contains(&sq, Point::new(0.5, 0.5)));
        assert!(!contains(&sq, Point::new(1.5, 0.5)));
    }
}
