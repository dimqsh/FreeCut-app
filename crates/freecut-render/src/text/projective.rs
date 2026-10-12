//! Projective (3 × 3 homography) maps and a resampler for glyph bitmaps:
//! the per-glyph transform of text animation (TX-023), including the 3D
//! turns about a unit's horizontal and vertical axes (TurnX, TurnY; FX-030's
//! perspective rule).
//!
//! Points are `(x, y)` in px, y down. A homography maps `(x, y, 1)` to
//! `(X, Y, W)` and the point to `(X / W, Y / W)`; points with `W` at or
//! below [`MIN_W`] lie at or behind the eye and have no image. Arithmetic is
//! sequential `f64`, so results are identical on every thread.

/// Points whose homogeneous `w` is at most this have no image (behind the
/// eye, or at infinity).
pub const MIN_W: f64 = 1e-3;

/// A 3 × 3 projective map, row-major: `[a b c; d e f; g h i]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Homography(pub [f64; 9]);

impl Default for Homography {
    fn default() -> Homography {
        Homography::IDENTITY
    }
}

impl Homography {
    pub const IDENTITY: Homography = Homography([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);

    pub fn translation(dx: f64, dy: f64) -> Homography {
        Homography([1.0, 0.0, dx, 0.0, 1.0, dy, 0.0, 0.0, 1.0])
    }

    pub fn scale(sx: f64, sy: f64) -> Homography {
        Homography([sx, 0.0, 0.0, 0.0, sy, 0.0, 0.0, 0.0, 1.0])
    }

    /// A clockwise turn on screen (y down) by `radians`.
    pub fn rotation(radians: f64) -> Homography {
        let (s, c) = radians.sin_cos();
        Homography([c, -s, 0.0, s, c, 0.0, 0.0, 0.0, 1.0])
    }

    /// A 3D turn of the plane about its vertical axis by `turn_y` and then
    /// about its horizontal axis by `turn_x` (degrees; positive turns move
    /// the right and the bottom edge away from the viewer), seen in
    /// perspective by an eye `eye` px in front of the plane, both axes
    /// through the origin. 90° is edge-on.
    pub fn turn(turn_x_deg: f64, turn_y_deg: f64, eye: f64) -> Homography {
        let (sx, cx) = turn_x_deg.to_radians().sin_cos();
        let (sy, cy) = turn_y_deg.to_radians().sin_cos();
        let eye = if eye.is_finite() && eye > 1e-6 {
            eye
        } else {
            1e-6
        };
        // (x, y, 0) turned about y: (x·cy, y, x·sy); then about x:
        // Y = y·cx − Z·sx, Z = y·sx + Z·cx. Projected: (X, Y)·eye / (eye + Z).
        Homography([
            cy,
            0.0,
            0.0,
            -sy * sx,
            cx,
            0.0,
            sy * cx / eye,
            sx / eye,
            1.0,
        ])
    }

    /// `self · other`: `other` first, then `self`.
    pub fn mul(&self, other: &Homography) -> Homography {
        let (a, b) = (&self.0, &other.0);
        let mut m = [0.0; 9];
        for r in 0..3 {
            for c in 0..3 {
                m[r * 3 + c] = a[r * 3] * b[c] + a[r * 3 + 1] * b[3 + c] + a[r * 3 + 2] * b[6 + c];
            }
        }
        Homography(m)
    }

    /// `self` followed by `next`.
    pub fn then(&self, next: &Homography) -> Homography {
        next.mul(self)
    }

    /// The image of `p`, `None` at or behind the eye.
    #[inline]
    pub fn apply(&self, p: (f64, f64)) -> Option<(f64, f64)> {
        let m = &self.0;
        let w = m[6] * p.0 + m[7] * p.1 + m[8];
        if w.is_nan() || w <= MIN_W {
            return None;
        }
        let x = (m[0] * p.0 + m[1] * p.1 + m[2]) / w;
        let y = (m[3] * p.0 + m[4] * p.1 + m[5]) / w;
        (x.is_finite() && y.is_finite()).then_some((x, y))
    }

    /// The inverse map, when `self` is invertible. For a point in front of
    /// the eye, the inverse gives its preimage with a positive `w` (the
    /// reciprocal of the forward `w`); a non-positive `w` marks a preimage
    /// behind the eye.
    pub fn inverse(&self) -> Option<Homography> {
        let m = &self.0;
        let (a, b, c, d, e, f, g, h, i) = (m[0], m[1], m[2], m[3], m[4], m[5], m[6], m[7], m[8]);
        let co = [
            e * i - f * h,
            c * h - b * i,
            b * f - c * e,
            f * g - d * i,
            a * i - c * g,
            c * d - a * f,
            d * h - e * g,
            b * g - a * h,
            a * e - b * d,
        ];
        let det = a * co[0] + b * co[3] + c * co[6];
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        let inv = co.map(|v| v / det);
        inv.iter().all(|v| v.is_finite()).then_some(Homography(inv))
    }

    /// Whether the bottom row is `[0 0 1]` (no perspective).
    pub fn is_affine(&self) -> bool {
        self.0[6] == 0.0 && self.0[7] == 0.0 && self.0[8] == 1.0
    }

    /// Whether the map is a pure translation.
    pub fn is_translation(&self) -> bool {
        self.is_affine()
            && self.0[0] == 1.0
            && self.0[1] == 0.0
            && self.0[3] == 0.0
            && self.0[4] == 1.0
    }
}

/// A resampled image: `width × height` pixels whose top-left pixel is at
/// `(x0, y0)` in the destination space.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Resampled {
    pub x0: i32,
    pub y0: i32,
    pub width: u32,
    pub height: u32,
    /// `channels` bytes per pixel (1: coverage, 4: premultiplied RGBA).
    pub data: Vec<u8>,
}

/// Largest side of a resampled image; larger results are dropped (a glyph
/// turned almost edge-on towards the eye).
pub(crate) const MAX_RESAMPLED_SIDE: u32 = 4096;

/// Maps a `width × height` image with `channels` bytes per pixel (1, or 4
/// premultiplied) through `to_dest` (source px → destination px, pixel
/// centres at `n + 0.5`) and samples it bilinearly at every destination
/// pixel centre whose preimage lies in front of the eye. `None` when the
/// image is empty, a corner has no image, or the result would exceed
/// [`MAX_RESAMPLED_SIDE`].
pub(crate) fn resample(
    src: &[u8],
    width: u32,
    height: u32,
    channels: usize,
    to_dest: &Homography,
) -> Option<Resampled> {
    if width == 0 || height == 0 || src.len() < (width * height) as usize * channels {
        return None;
    }
    let inverse = to_dest.inverse()?;
    let (w, h) = (f64::from(width), f64::from(height));
    let mut b = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for corner in [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)] {
        let (x, y) = to_dest.apply(corner)?;
        b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
    }
    // One pixel of margin for the bilinear footprint.
    let x0 = (b.0.floor() - 1.0) as i64;
    let y0 = (b.1.floor() - 1.0) as i64;
    let x1 = (b.2.ceil() + 1.0) as i64;
    let y1 = (b.3.ceil() + 1.0) as i64;
    let (dw, dh) = (x1 - x0, y1 - y0);
    let max = i64::from(MAX_RESAMPLED_SIDE);
    if dw <= 0 || dh <= 0 || dw > max || dh > max {
        return None;
    }
    let (dw, dh) = (dw as usize, dh as usize);
    let mut out = vec![0u8; dw * dh * channels];
    let m = &inverse.0;
    let (sw, sh) = (width as i64, height as i64);
    for row in 0..dh {
        let py = (y0 + row as i64) as f64 + 0.5;
        for col in 0..dw {
            let px = (x0 + col as i64) as f64 + 0.5;
            let wq = m[6] * px + m[7] * py + m[8];
            // Preimages behind the eye (w ≤ 0) are not part of the image.
            if wq.is_nan() || wq <= 1e-12 {
                continue;
            }
            let u = (m[0] * px + m[1] * py + m[2]) / wq - 0.5;
            let v = (m[3] * px + m[4] * py + m[5]) / wq - 0.5;
            if !(u > -1.0 && v > -1.0 && u < w && v < h) {
                continue;
            }
            let (fu, fv) = (u.floor(), v.floor());
            let (tx, ty) = (u - fu, v - fv);
            let (iu, iv) = (fu as i64, fv as i64);
            let dst = (row * dw + col) * channels;
            for k in 0..channels {
                let tap = |x: i64, y: i64| -> f64 {
                    if x < 0 || y < 0 || x >= sw || y >= sh {
                        0.0
                    } else {
                        f64::from(src[(y as usize * width as usize + x as usize) * channels + k])
                    }
                };
                let top = tap(iu, iv) * (1.0 - tx) + tap(iu + 1, iv) * tx;
                let bottom = tap(iu, iv + 1) * (1.0 - tx) + tap(iu + 1, iv + 1) * tx;
                let value = top * (1.0 - ty) + bottom * ty;
                out[dst + k] = value.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Some(Resampled {
        x0: x0 as i32,
        y0: y0 as i32,
        width: dw as u32,
        height: dh as u32,
        data: out,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9
    }

    #[test]
    fn rotation_is_clockwise_on_screen() {
        let r = Homography::rotation(std::f64::consts::FRAC_PI_2);
        assert!(close(r.apply((1.0, 0.0)).unwrap(), (0.0, 1.0)));
    }

    #[test]
    fn inverse_undoes_a_projective_map() {
        let m = Homography::translation(5.0, -3.0)
            .mul(&Homography::rotation(0.3))
            .mul(&Homography::turn(20.0, -35.0, 120.0))
            .mul(&Homography::scale(1.5, 0.75));
        let inv = m.inverse().unwrap();
        for p in [(0.0, 0.0), (10.0, -4.0), (-25.0, 30.0)] {
            let q = m.apply(p).unwrap();
            assert!(close(inv.apply(q).unwrap(), p), "{p:?}");
        }
    }

    #[test]
    fn a_turn_foreshortens_and_puts_the_far_side_smaller() {
        // Turned 60° about the vertical axis: the right half goes away.
        let t = Homography::turn(0.0, 60.0, 100.0);
        let right = t.apply((50.0, 50.0)).unwrap();
        let left = t.apply((-50.0, 50.0)).unwrap();
        assert!(right.0 < 25.0 && right.0 > 0.0, "{right:?}");
        assert!(left.0 > -50.0 && left.0 < -25.0, "{left:?}");
        // The far edge is shorter than the near one.
        assert!(right.1 < left.1, "{right:?} {left:?}");
        // Edge-on at 90°.
        let edge = Homography::turn(0.0, 90.0, 100.0);
        assert!(edge.apply((40.0, 10.0)).unwrap().0.abs() < 1e-9);
        let flat = Homography::turn(90.0, 0.0, 100.0);
        assert!(flat.apply((40.0, 10.0)).unwrap().1.abs() < 1e-9);
    }

    #[test]
    fn points_behind_the_eye_have_no_image() {
        let t = Homography::turn(0.0, -80.0, 10.0);
        assert!(t.apply((100.0, 0.0)).is_none());
    }

    #[test]
    fn identity_resampling_copies_the_image() {
        let src: Vec<u8> = (0..12).map(|v| v * 20).collect();
        let r = resample(&src, 4, 3, 1, &Homography::translation(10.0, 20.0)).unwrap();
        for y in 0..3 {
            for x in 0..4 {
                let dx = (x + 10 - r.x0) as usize;
                let dy = (y + 20 - r.y0) as usize;
                assert_eq!(
                    r.data[dy * r.width as usize + dx],
                    src[(y * 4 + x) as usize],
                    "{x},{y}"
                );
            }
        }
        // The margin stays empty.
        assert_eq!(r.data[0], 0);
    }

    #[test]
    fn resampling_keeps_coverage_under_a_turn() {
        let src = vec![255u8; 40 * 40];
        let to_dest = Homography::translation(100.0, 100.0)
            .mul(&Homography::turn(0.0, 45.0, 60.0))
            .mul(&Homography::translation(-20.0, -20.0));
        let r = resample(&src, 40, 40, 1, &to_dest).unwrap();
        let area: f64 = r.data.iter().map(|&v| f64::from(v) / 255.0).sum();
        // The exact projected area of the square.
        let corners: Vec<(f64, f64)> = [(0.0, 0.0), (40.0, 0.0), (40.0, 40.0), (0.0, 40.0)]
            .iter()
            .map(|&p| to_dest.apply(p).unwrap())
            .collect();
        let mut exact = 0.0;
        for i in 0..4 {
            let (a, b) = (corners[i], corners[(i + 1) % 4]);
            exact += a.0 * b.1 - b.0 * a.1;
        }
        let exact = exact.abs() / 2.0;
        assert!((area - exact).abs() / exact < 0.05, "{area} vs {exact}");
    }
}
