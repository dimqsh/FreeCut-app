//! Shape-mask coverage on the CPU (TR-017, TR-018, TR-020; CL-031): the
//! shared core of colour masks and, later, clip masks.
//!
//! Every shape is a signed distance `d` in units of the item's shorter side
//! (negative inside), in the mask's own frame: the point relative to the
//! mask's centre, turned back by its rotation. Then:
//! - Expand (colour masks) moves the edge: `d − expand/100 · 0.1`;
//! - feather turns distance into coverage with a smoothstep centred on the
//!   edge, `c = 1 − smoothstep(−f/2, f/2, d)`, where `f` is the feather
//!   (0–25 % of the shorter side) but never less than one pixel, so a hard
//!   edge is still antialiased;
//! - a shape with no area covers nothing;
//! - invert takes `1 − c`;
//! - masks combine in list order ([`MaskCombine`]): Add = max, Subtract =
//!   `c·(1 − m)`, Intersect = min, starting from nothing when the first mask
//!   adds and from everything otherwise.
//!
//! The distances are derived from the shapes' geometry in-house (ellipse:
//! the first-order estimate `f / |∇f|` of `f = |p / r| − 1`, exact on the
//! edge; rounded rectangle: the distance to the inset box minus the corner
//! radius). Plain `f32` arithmetic; the rotation's sine and cosine come from
//! `libm`, so coverage is the same on every platform.

use freecut_timeline::glam::{DMat3, DVec2};
use freecut_timeline::model::{
    ColorMask, EXPAND_LIMIT, MASK_FEATHER_SPAN, MASK_UNIT, MaskCombine, MaskShape, Rotation,
};

/// How far Expand ±100 moves the edge: 10 % of the shorter side (CL-031).
const EXPAND_SPAN: f32 = 0.1;

/// One mask ready to evaluate: its numbers in shorter sides, as `f32`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Compiled {
    shape: Shape,
    center: [f32; 2],
    cos: f32,
    sin: f32,
    /// The edge offset of Expand, in shorter sides.
    grow: f32,
    feather: f32,
    invert: bool,
    mode: MaskCombine,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Shape {
    /// No area: covers nothing.
    Empty,
    Linear,
    Band {
        half: f32,
    },
    Ellipse {
        rx: f32,
        ry: f32,
    },
    Rect {
        bx: f32,
        by: f32,
        radius: f32,
    },
}

impl Compiled {
    fn of(m: &ColorMask) -> Compiled {
        let unit = MASK_UNIT as f32;
        let mask = &m.mask;
        let (w, h) = (mask.size.x as f32 / unit, mask.size.y as f32 / unit);
        let shape = if mask.is_empty_area() {
            Shape::Empty
        } else {
            match mask.shape {
                MaskShape::Linear => Shape::Linear,
                MaskShape::Band { width } => Shape::Band {
                    half: f32::from(width.get()) / 1000.0 / 2.0,
                },
                MaskShape::Ellipse => Shape::Ellipse {
                    rx: w / 2.0,
                    ry: h / 2.0,
                },
                MaskShape::Rectangle { roundness } => {
                    let (bx, by) = (w / 2.0, h / 2.0);
                    Shape::Rect {
                        bx,
                        by,
                        radius: f32::from(roundness.get()) / 1000.0 * bx.min(by),
                    }
                }
            }
        };
        let angle = (mask.rotation.hundredths() as f64 / 100.0).to_radians();
        Compiled {
            shape,
            center: [mask.center.x as f32 / unit, mask.center.y as f32 / unit],
            cos: libm::cos(angle) as f32,
            sin: libm::sin(angle) as f32,
            grow: f32::from(m.expand.clamp(-EXPAND_LIMIT, EXPAND_LIMIT)) / 100.0 * EXPAND_SPAN,
            feather: f32::from(mask.feather.get()) / 1000.0 * MASK_FEATHER_SPAN as f32,
            invert: mask.invert,
            mode: m.mode,
        }
    }

    /// The signed distance at `p` (shorter sides from the item's centre);
    /// `None` for a shape with no area.
    fn distance(&self, p: [f32; 2]) -> Option<f32> {
        let (dx, dy) = (p[0] - self.center[0], p[1] - self.center[1]);
        // Turned back by the rotation (clockwise on screen, y down).
        let qx = self.cos * dx + self.sin * dy;
        let qy = -self.sin * dx + self.cos * dy;
        Some(match self.shape {
            Shape::Empty => return None,
            // The side above the edge is kept (TR-017).
            Shape::Linear => qy,
            Shape::Band { half } => qy.abs() - half,
            Shape::Ellipse { rx, ry } => {
                let (ux, uy) = (qx / rx, qy / ry);
                let k = (ux * ux + uy * uy).sqrt();
                let (gx, gy) = (ux / rx, uy / ry);
                let g = (gx * gx + gy * gy).sqrt();
                if k <= f32::EPSILON || g <= f32::EPSILON {
                    -rx.min(ry)
                } else {
                    // f / |∇f| with f = k − 1 and ∇f = (gx, gy) / k.
                    (k - 1.0) * k / g
                }
            }
            Shape::Rect { bx, by, radius } => {
                let ex = qx.abs() - bx + radius;
                let ey = qy.abs() - by + radius;
                let outside = (ex.max(0.0) * ex.max(0.0) + ey.max(0.0) * ey.max(0.0)).sqrt();
                outside + ex.max(ey).min(0.0) - radius
            }
        })
    }

    /// Feathered, inverted coverage at `p`; `aa` is one pixel in shorter
    /// sides.
    fn coverage(&self, p: [f32; 2], aa: f32) -> f32 {
        let c = match self.distance(p) {
            None => 0.0,
            Some(d) => {
                let f = self.feather.max(aa);
                1.0 - smoothstep(-f / 2.0, f / 2.0, d - self.grow)
            }
        };
        if self.invert { 1.0 - c } else { c }
    }
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The coverage of one mask at `point` (shorter sides from the item's
/// centre, +y down) on an item `item_px` pixels big at the grading
/// resolution: 0 outside, 1 inside, the feather between.
pub fn coverage(mask: &ColorMask, item_px: (f32, f32), point: [f32; 2]) -> f32 {
    Compiled::of(mask).coverage(point, pixel(item_px))
}

/// One pixel in shorter sides of an item `item_px` big.
fn pixel(item_px: (f32, f32)) -> f32 {
    1.0 / item_px.0.min(item_px.1).max(1.0)
}

/// A list of masks ready to evaluate, combined in list order.
#[derive(Debug, Clone, PartialEq)]
pub struct MaskSet {
    masks: Vec<Compiled>,
}

impl MaskSet {
    pub fn new(masks: &[ColorMask]) -> MaskSet {
        MaskSet {
            masks: masks.iter().map(Compiled::of).collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.masks.is_empty()
    }

    /// The combined coverage at `point` with pixels `aa` shorter sides
    /// big; 1 (the whole item) without masks.
    pub fn coverage(&self, point: [f32; 2], aa: f32) -> f32 {
        let Some(first) = self.masks.first() else {
            return 1.0;
        };
        self.masks
            .iter()
            .fold(first.mode.start(), |acc, m| {
                m.mode.join(acc, m.coverage(point, aa))
            })
            .clamp(0.0, 1.0)
    }
}

/// The combined coverage of `masks` at `point`, like [`coverage`].
pub fn combined(masks: &[ColorMask], item_px: (f32, f32), point: [f32; 2]) -> f32 {
    MaskSet::new(masks).coverage(point, pixel(item_px))
}

/// Where the pixels of a buffer sit in an item's mask frame: an affine map
/// from a pixel's centre to shorter sides from the item's centre, and the
/// size of one pixel there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaskSpace {
    /// Row-major `[a, b, c, d, e, f]`: `x' = a·x + b·y + c`, `y' = d·x + e·y + f`.
    m: [f32; 6],
    aa: f32,
}

impl MaskSpace {
    /// A buffer that is the item itself, `width` × `height` pixels: the
    /// canvas of an adjustment clip at output resolution (CL-031).
    pub fn canvas(width: u32, height: u32) -> MaskSpace {
        let (w, h) = (f64::from(width.max(1)), f64::from(height.max(1)));
        let s = w.min(h);
        MaskSpace::from_matrix(DMat3::from_cols_array(&[
            1.0 / s,
            0.0,
            0.0,
            0.0,
            1.0 / s,
            0.0,
            -w / 2.0 / s,
            -h / 2.0 / s,
            1.0,
        ]))
    }

    /// A clip's stored picture (`stored` pixels, shown turned by `rotation`
    /// after `mirrored`) shown in a display frame `display` big: masks live
    /// in the display frame (TR-017's clip space), so the picture's pixels
    /// are mapped there first, undoing rotation, mirroring and pixel
    /// aspect like the compositor does.
    pub fn picture(
        stored: (u32, u32),
        rotation: Rotation,
        mirrored: bool,
        display: (u32, u32),
    ) -> MaskSpace {
        let (w, h) = (f64::from(stored.0.max(1)), f64::from(stored.1.max(1)));
        let (dw, dh) = (f64::from(display.0.max(1)), f64::from(display.1.max(1)));
        let turned = matches!(rotation, Rotation::Cw90 | Rotation::Cw270);
        let (uw, uh) = if turned { (h, w) } else { (w, h) };
        let to_upright = DMat3::from_scale(DVec2::new(uw / dw, uh / dh));
        let unrotate = match rotation {
            Rotation::None => DMat3::IDENTITY,
            Rotation::Cw90 => DMat3::from_cols_array(&[0.0, -1.0, 0.0, 1.0, 0.0, 0.0, 0.0, h, 1.0]),
            Rotation::Cw180 => DMat3::from_cols_array(&[-1.0, 0.0, 0.0, 0.0, -1.0, 0.0, w, h, 1.0]),
            Rotation::Cw270 => {
                DMat3::from_cols_array(&[0.0, 1.0, 0.0, -1.0, 0.0, 0.0, w, 0.0, 1.0])
            }
        };
        let unmirror = if mirrored {
            DMat3::from_cols_array(&[-1.0, 0.0, 0.0, 0.0, 1.0, 0.0, w, 0.0, 1.0])
        } else {
            DMat3::IDENTITY
        };
        let display_to_stored = unmirror * unrotate * to_upright;
        let s = dw.min(dh);
        let display_to_mask = DMat3::from_cols_array(&[
            1.0 / s,
            0.0,
            0.0,
            0.0,
            1.0 / s,
            0.0,
            -dw / 2.0 / s,
            -dh / 2.0 / s,
            1.0,
        ]);
        MaskSpace::from_matrix(display_to_mask * display_to_stored.inverse())
    }

    fn from_matrix(m: DMat3) -> MaskSpace {
        let a = m.to_cols_array();
        // Column-major: x' = a0·x + a3·y + a6, y' = a1·x + a4·y + a7.
        let aa = DVec2::new(a[0], a[1])
            .length()
            .max(DVec2::new(a[3], a[4]).length());
        MaskSpace {
            m: [a[0], a[3], a[6], a[1], a[4], a[7]].map(|v| v as f32),
            aa: aa as f32,
        }
    }

    /// The centre of pixel (`x`, `y`) in shorter sides.
    #[inline]
    pub fn point(&self, x: u32, y: u32) -> [f32; 2] {
        let (x, y) = (x as f32 + 0.5, y as f32 + 0.5);
        let m = &self.m;
        [m[0] * x + m[1] * y + m[2], m[3] * x + m[4] * y + m[5]]
    }

    /// One pixel in shorter sides.
    pub fn pixel(&self) -> f32 {
        self.aa
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use freecut_timeline::model::{Angle, Mask, MaskPoint, MaskShapeKind, PerMille};

    fn mask(kind: MaskShapeKind) -> ColorMask {
        ColorMask::new(1, Mask::new(kind))
    }

    #[test]
    fn shapes_cover_their_inside() {
        let px = (1000.0, 1000.0);
        let e = mask(MaskShapeKind::Ellipse);
        assert_eq!(coverage(&e, px, [0.0, 0.0]), 1.0);
        assert_eq!(coverage(&e, px, [0.4, 0.0]), 0.0);
        assert!((coverage(&e, px, [0.25, 0.0]) - 0.5).abs() < 1e-3, "edge");
        let l = mask(MaskShapeKind::Linear);
        assert_eq!(coverage(&l, px, [0.0, -0.1]), 1.0, "above is kept");
        assert_eq!(coverage(&l, px, [0.0, 0.1]), 0.0);
        let b = mask(MaskShapeKind::Band);
        assert_eq!(coverage(&b, px, [3.0, 0.2]), 1.0);
        assert_eq!(coverage(&b, px, [0.0, 0.3]), 0.0);
        let r = mask(MaskShapeKind::Rectangle);
        assert_eq!(coverage(&r, px, [0.29, 0.19]), 1.0);
        assert_eq!(coverage(&r, px, [0.31, 0.0]), 0.0);
        assert_eq!(coverage(&r, px, [0.0, 0.21]), 0.0);
    }

    #[test]
    fn rotation_turns_the_shape_clockwise() {
        let mut l = mask(MaskShapeKind::Linear);
        l.mask.rotation = Angle::from_degrees(90);
        // Turned a quarter clockwise, the kept side faces right.
        let px = (1000.0, 1000.0);
        assert_eq!(coverage(&l, px, [0.1, 0.0]), 1.0);
        assert_eq!(coverage(&l, px, [-0.1, 0.0]), 0.0);
    }

    #[test]
    fn rounded_corners_cut_the_corner() {
        let mut r = mask(MaskShapeKind::Rectangle);
        let px = (1000.0, 1000.0);
        let corner = [0.295, 0.195];
        assert_eq!(coverage(&r, px, corner), 1.0);
        r.mask.shape = MaskShape::Rectangle {
            roundness: PerMille::ONE,
        };
        assert_eq!(coverage(&r, px, corner), 0.0);
    }

    #[test]
    fn zero_area_covers_nothing_and_inverted_everything() {
        let mut e = mask(MaskShapeKind::Ellipse);
        e.mask.size = MaskPoint::new(0, 5_000);
        e.mask.feather = PerMille::ONE;
        assert_eq!(coverage(&e, (100.0, 100.0), [0.0, 0.0]), 0.0);
        e.mask.invert = true;
        assert_eq!(coverage(&e, (100.0, 100.0), [0.0, 0.0]), 1.0);
    }

    #[test]
    fn spaces_map_pixel_centres() {
        let s = MaskSpace::canvas(200, 100);
        let c = s.point(100, 50);
        assert!(
            (c[0] - 0.005).abs() < 1e-6 && (c[1] - 0.005).abs() < 1e-6,
            "{c:?}"
        );
        assert!((s.pixel() - 0.01).abs() < 1e-7);
        // A picture stored sideways shows upright: its stored top-left is
        // the display's top-right after a quarter turn clockwise.
        let p = MaskSpace::picture((100, 200), Rotation::Cw90, false, (200, 100));
        let tl = p.point(0, 0);
        assert!(
            (tl[0] - 0.995).abs() < 1e-4 && (tl[1] + 0.495).abs() < 1e-4,
            "{tl:?}"
        );
    }
}
