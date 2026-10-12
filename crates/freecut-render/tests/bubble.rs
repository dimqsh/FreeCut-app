//! Bubble outlines (TX-055): closed, simple, the tail's base on the body,
//! and the Shout burst reproducible from its seed.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

use freecut_render::shape::bubble::{
    BubbleKind, BubbleSpec, Side, Tail, bubble_body, bubble_path, tail_base,
};
use freecut_render::shape::{Path, Point, Rect};

const KINDS: [BubbleKind; 9] = [
    BubbleKind::Rounded,
    BubbleKind::Oval,
    BubbleKind::Thought,
    BubbleKind::Shout,
    BubbleKind::Square,
    BubbleKind::Tag,
    BubbleKind::ArrowTag { point: Side::Left },
    BubbleKind::ArrowTag { point: Side::Right },
    BubbleKind::Ticket,
];

/// A text box of 300 × 120 at 50 px per em.
const EM: f32 = 50.0;

fn text_box() -> Rect {
    Rect::new(0.0, 0.0, 300.0, 120.0)
}

fn spec(kind: BubbleKind, tail: Option<Tail>, seed: u32) -> BubbleSpec {
    BubbleSpec {
        kind,
        tail,
        seed,
        ..BubbleSpec::default()
    }
}

fn body(kind: BubbleKind) -> Rect {
    bubble_body(kind, text_box(), (0.45 * EM, 0.3 * EM), EM)
}

fn cross(o: Point, a: Point, b: Point) -> f64 {
    let (ax, ay) = (f64::from(a.x - o.x), f64::from(a.y - o.y));
    let (bx, by) = (f64::from(b.x - o.x), f64::from(b.y - o.y));
    ax * by - ay * bx
}

/// Whether segments `ab` and `cd` cross or touch.
fn segments_meet(a: Point, b: Point, c: Point, d: Point) -> bool {
    let d1 = cross(c, d, a);
    let d2 = cross(c, d, b);
    let d3 = cross(a, b, c);
    let d4 = cross(a, b, d);
    let eps = 1e-9;
    if ((d1 > eps && d2 < -eps) || (d1 < -eps && d2 > eps))
        && ((d3 > eps && d4 < -eps) || (d3 < -eps && d4 > eps))
    {
        return true;
    }
    // Touching (collinear overlap or an end on the other segment).
    let on = |p: Point, q: Point, r: Point| {
        cross(p, q, r).abs() <= eps
            && r.x >= p.x.min(q.x) - 1e-6
            && r.x <= p.x.max(q.x) + 1e-6
            && r.y >= p.y.min(q.y) - 1e-6
            && r.y <= p.y.max(q.y) + 1e-6
    };
    on(c, d, a) || on(c, d, b) || on(a, b, c) || on(a, b, d)
}

/// Panics unless every contour is a closed polygon (at least three
/// distinct points, non-zero area) without self-intersections.
fn assert_simple(path: &Path, what: &str) {
    assert!(!path.contours.is_empty(), "{what}: empty");
    for (k, c) in path.contours.iter().enumerate() {
        assert!(c.len() >= 3, "{what}: contour {k} has {} points", c.len());
        assert!(
            c.iter().all(|p| p.x.is_finite() && p.y.is_finite()),
            "{what}: contour {k} has non-finite points"
        );
        let area: f64 = (0..c.len())
            .map(|i| {
                let (p, q) = (c[i], c[(i + 1) % c.len()]);
                f64::from(p.x) * f64::from(q.y) - f64::from(q.x) * f64::from(p.y)
            })
            .sum();
        assert!(area.abs() > 1.0, "{what}: contour {k} has no area");
        let n = c.len();
        for i in 0..n {
            assert!(
                c[i].distance(c[(i + 1) % n]) > 0.0,
                "{what}: contour {k} repeats point {i}"
            );
        }
        // Sweep over x: only segments whose x ranges overlap are compared.
        let mut order: Vec<usize> = (0..n).collect();
        let span = |i: usize| {
            let (a, b) = (c[i], c[(i + 1) % n]);
            (a.x.min(b.x), a.x.max(b.x), a.y.min(b.y), a.y.max(b.y))
        };
        order.sort_by(|&a, &b| span(a).0.total_cmp(&span(b).0));
        for (oi, &i) in order.iter().enumerate() {
            let si = span(i);
            for &j in &order[oi + 1..] {
                let sj = span(j);
                if sj.0 > si.1 + 1e-6 {
                    break;
                }
                let adjacent = (i + 1) % n == j || (j + 1) % n == i;
                if adjacent || sj.2 > si.3 + 1e-6 || sj.3 < si.2 - 1e-6 {
                    continue;
                }
                let (a, b) = (c[i], c[(i + 1) % n]);
                let (p, q) = (c[j], c[(j + 1) % n]);
                assert!(
                    !segments_meet(a, b, p, q),
                    "{what}: contour {k} crosses itself at edges {i} and {j}: {a:?}-{b:?} / {p:?}-{q:?}"
                );
            }
        }
    }
}

/// Distance from `p` to the closed polygon's outline.
fn distance_to_outline(contour: &[Point], p: Point) -> f32 {
    (0..contour.len())
        .map(|i| {
            let (a, b) = (contour[i], contour[(i + 1) % contour.len()]);
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let len2 = dx * dx + dy * dy;
            let t = if len2 > 0.0 {
                (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            p.distance(Point::new(a.x + t * dx, a.y + t * dy))
        })
        .fold(f32::INFINITY, f32::min)
}

fn tips() -> Vec<Point> {
    let default = Tail::default_for((300.0, 120.0), EM).tip;
    vec![
        default,
        Point::new(0.0, 160.0),
        Point::new(260.0, 20.0),
        Point::new(-240.0, -150.0),
        Point::new(40.0, -130.0),
        Point::new(200.0, 140.0),
        // Grazing an edge, and beyond the far side.
        Point::new(-165.0, 100.0),
        Point::new(-400.0, 200.0),
        Point::new(-600.0, 260.0),
        Point::new(500.0, -20.0),
    ]
}

#[test]
fn every_outline_is_closed_and_simple_over_the_bend_range() {
    for kind in KINDS {
        let b = body(kind);
        for roundness in [0.0, 0.4, 1.0] {
            assert_simple(
                &bubble_path(&spec(kind, None, 1), b, roundness),
                &format!("{kind:?} no tail r{roundness}"),
            );
        }
        for tip in tips() {
            for step in -10..=10 {
                let bend = step as f32 / 10.0;
                for base in [0.1, 0.24, 0.6] {
                    let tail = Tail { tip, base, bend };
                    let path = bubble_path(&spec(kind, Some(tail), 3), b, 0.4);
                    assert_simple(
                        &path,
                        &format!("{kind:?} tip {tip:?} bend {bend} base {base}"),
                    );
                }
            }
        }
    }
}

#[test]
fn the_tail_base_lies_on_the_body_and_the_tip_is_reached() {
    for kind in KINDS.iter().copied().filter(|k| k.has_tail()) {
        let b = body(kind);
        let plain = bubble_path(&spec(kind, None, 5), b, 0.4);
        let outline = &plain.contours[0];
        for tip in tips() {
            let tail = Tail {
                tip,
                base: 0.24,
                bend: 0.3,
            };
            let s = spec(kind, Some(tail), 5);
            let (a, c) = tail_base(&s, b, 0.4).expect("tip outside the body");
            for p in [a, c] {
                let d = distance_to_outline(outline, p);
                assert!(
                    d < 0.05,
                    "{kind:?} {tip:?}: base end {p:?} is {d} off the body"
                );
            }
            // The tail's two ends are about the base width apart.
            let short = b.width().min(b.height());
            let gap = a.distance(c);
            assert!(
                gap > 0.1 * short && gap < 0.7 * short,
                "{kind:?} {tip:?}: {gap}"
            );
            // The outline passes through the tip.
            let centre = b.center();
            let abs_tip = Point::new(centre.x + tip.x, centre.y + tip.y);
            let path = bubble_path(&s, b, 0.4);
            assert!(
                path.contours[0].iter().any(|p| p.distance(abs_tip) < 1e-3),
                "{kind:?}: tip {tip:?} not on the outline"
            );
        }
    }
}

#[test]
fn the_tail_base_has_its_width_and_sits_nearest_the_tip() {
    // A wide one-line caption: 900 × 40 text at 50 px per em. TX-055's
    // base is 24 % of the body's shorter side wherever the tip is, and a
    // tip beyond the far side leaves from the near end of the body.
    let text = Rect::new(0.0, 0.0, 900.0, 40.0);
    for kind in KINDS.iter().copied().filter(|k| k.has_tail()) {
        let b = bubble_body(kind, text, (0.45 * EM, 0.3 * EM), EM);
        let (hw, hh) = (b.width() / 2.0, b.height() / 2.0);
        let short = b.width().min(b.height());
        let tips = [
            Tail::default_for((900.0, 40.0), EM).tip,
            Point::new(0.0, hh + 1.2 * EM),
            Point::new(-0.97 * hw, hh + 0.3 * short),
            Point::new(-1.5 * hw, 4.0 * hh),
            Point::new(-2.1 * hw, 5.3 * hh),
            Point::new(1.3 * hw, -0.2 * hh),
        ];
        for tip in tips {
            let s = spec(
                kind,
                Some(Tail {
                    tip,
                    base: 0.24,
                    bend: 0.2,
                }),
                9,
            );
            let (a, c) = tail_base(&s, b, 0.4).expect("tip outside the body");
            let gap = a.distance(c);
            let what = format!("{kind:?} tip {tip:?}: base {gap} of {}", 0.24 * short);
            // A Shout widens its base to the neighbouring valleys, up to
            // the widest base.
            let most = if kind == BubbleKind::Shout {
                0.6
            } else {
                0.245
            };
            assert!(gap >= 0.2 * short && gap <= most * short, "{what}");
            for bend in [-1.0, 0.0, 0.2, 1.0] {
                let tail = Tail {
                    tip,
                    base: 0.24,
                    bend,
                };
                assert_simple(&bubble_path(&spec(kind, Some(tail), 9), b, 0.4), &what);
            }
            // Beyond an end, the base sits at that end of the outline.
            if tip.x.abs() > hw {
                let plain = bubble_path(&spec(kind, None, 9), b, 0.4);
                let xs = plain.contours[0].iter().map(|p| p.x);
                let (x0, x1) = xs.fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), x| {
                    (lo.min(x), hi.max(x))
                });
                let end = if tip.x < 0.0 { x0 } else { x1 };
                let mid = (a.x + c.x) / 2.0;
                assert!(
                    (mid - end).abs() < 0.15 * (x1 - x0),
                    "{what}: base centre x {mid} far from the tip's end {end}"
                );
            }
        }
    }
}

#[test]
fn a_tip_inside_the_body_hides_the_tail() {
    for kind in [BubbleKind::Rounded, BubbleKind::Oval, BubbleKind::Shout] {
        let b = body(kind);
        let inside = Tail {
            tip: Point::new(10.0, 5.0),
            base: 0.24,
            bend: 0.2,
        };
        let s = spec(kind, Some(inside), 2);
        assert_eq!(tail_base(&s, b, 0.4), None);
        assert_eq!(
            bubble_path(&s, b, 0.4),
            bubble_path(&spec(kind, None, 2), b, 0.4)
        );
    }
}

#[test]
fn the_same_seed_gives_the_same_burst() {
    let b = body(BubbleKind::Shout);
    let shout = |seed| bubble_path(&spec(BubbleKind::Shout, None, seed), b, 0.0);
    assert_eq!(shout(42), shout(42));
    assert_ne!(shout(42), shout(43));
    // Without irregularity every seed draws the same regular burst.
    let regular = |seed| {
        bubble_path(
            &BubbleSpec {
                jitter: 0.0,
                ..spec(BubbleKind::Shout, None, seed)
            },
            b,
            0.0,
        )
    };
    assert_eq!(regular(1), regular(2));
    // The spikes reach out by the spike depth (18 % of the shorter side).
    let reach = shout(42).bounds().unwrap();
    assert!(reach.width() > b.width());
}

#[test]
fn a_thought_bubble_trails_circles_to_the_tip() {
    let b = body(BubbleKind::Thought);
    for trail in 2..=4u8 {
        let s = BubbleSpec {
            trail,
            ..spec(
                BubbleKind::Thought,
                Some(Tail {
                    tip: Point::new(-150.0, 220.0),
                    base: 0.24,
                    bend: 0.0,
                }),
                0,
            )
        };
        let path = bubble_path(&s, b, 0.4);
        assert_eq!(path.contours.len(), 1 + usize::from(trail));
        // Circles shrink towards the tip.
        let size = |c: &Vec<Point>| {
            let p = Path {
                contours: vec![c.clone()],
            };
            p.bounds().unwrap().width()
        };
        let sizes: Vec<f32> = path.contours[1..].iter().map(size).collect();
        assert!(sizes.windows(2).all(|w| w[1] < w[0]), "{sizes:?}");
    }
}

#[test]
fn elliptic_bodies_hold_the_text_corners_and_bodies_have_a_minimum() {
    let text = text_box();
    for kind in [BubbleKind::Oval, BubbleKind::Thought, BubbleKind::Shout] {
        let b = bubble_body(kind, text, (0.0, 0.0), EM);
        let c = b.center();
        let (rx, ry) = (b.width() / 2.0, b.height() / 2.0);
        for corner in [(0.0, 0.0), (300.0, 0.0), (0.0, 120.0), (300.0, 120.0)] {
            let v = ((corner.0 - c.x) / rx).powi(2) + ((corner.1 - c.y) / ry).powi(2);
            assert!(v <= 1.0 + 1e-5, "{kind:?}: {corner:?} outside ({v})");
        }
    }
    // A single letter still gets a 2 em × 1.5 em body.
    let tiny = bubble_body(
        BubbleKind::Rounded,
        Rect::new(0.0, 0.0, 10.0, 10.0),
        (0.0, 0.0),
        EM,
    );
    assert!(tiny.width() >= 2.0 * EM - 1e-3 && tiny.height() >= 1.5 * EM - 1e-3);
    assert_eq!(tiny.center(), Point::new(5.0, 5.0));
}

#[test]
fn the_simplicity_check_catches_a_crossing() {
    let bow_tie = Path {
        contours: vec![vec![
            Point::new(0.0, 0.0),
            Point::new(10.0, 10.0),
            Point::new(10.0, 0.0),
            Point::new(0.0, 10.0),
        ]],
    };
    let caught = std::panic::catch_unwind(|| assert_simple(&bow_tie, "bow tie"));
    assert!(caught.is_err());
}
