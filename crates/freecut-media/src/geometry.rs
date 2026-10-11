//! Display geometry: rotation and mirroring from FFmpeg display matrices,
//! applying them to RGBA pixels, and size fitting. Pure functions.
//!
//! Convention: a picture is displayed by first mirroring the coded picture
//! horizontally (when `mirrored`), then rotating it clockwise by
//! [`Rotation`].
//!
//! FFmpeg's display matrix maps a coded point `(p, q)` to the displayed point
//! `(a·p + c·q, b·p + d·q)` with `a, b, c, d = m[0], m[1], m[3], m[4]`, in
//! image coordinates (y down). So the coded x axis lands on `(a, b)` and the
//! y axis on `(c, d)`. We pick the one of the eight mirror/quarter-turn
//! combinations whose axes are closest to those images, which also rounds
//! non-quarter angles to the nearest quarter turn.

use freecut_timeline::model::Rotation;

const ROTATIONS: [Rotation; 4] = [
    Rotation::None,
    Rotation::Cw90,
    Rotation::Cw180,
    Rotation::Cw270,
];

/// Clockwise quarter turns of `r`.
pub(crate) fn quarter_turns(r: Rotation) -> u8 {
    match r {
        Rotation::None => 0,
        Rotation::Cw90 => 1,
        Rotation::Cw180 => 2,
        Rotation::Cw270 => 3,
    }
}

/// Where the coded x and y unit axes land for `(rotation, mirrored)`.
fn axes(turns: u8, mirrored: bool) -> [(f64, f64); 2] {
    // Clockwise in y-down coordinates: (x, y) → (-y, x).
    let rot = |(x, y): (f64, f64)| (-y, x);
    let mut ex = if mirrored { (-1.0, 0.0) } else { (1.0, 0.0) };
    let mut ey = (0.0, 1.0);
    for _ in 0..turns {
        ex = rot(ex);
        ey = rot(ey);
    }
    [ex, ey]
}

/// The orientation a display matrix describes. A degenerate matrix (zero
/// or non-finite axes) means "as coded".
pub(crate) fn orientation_from_matrix(m: &[i32; 9]) -> (Rotation, bool) {
    let unit = |x: i32, y: i32| {
        let (x, y) = (f64::from(x), f64::from(y));
        let len = x.hypot(y);
        (len > 0.0 && len.is_finite()).then(|| (x / len, y / len))
    };
    let (Some(ex), Some(ey)) = (unit(m[0], m[1]), unit(m[3], m[4])) else {
        return (Rotation::None, false);
    };
    let mut best = (Rotation::None, false);
    let mut best_score = f64::NEG_INFINITY;
    for mirrored in [false, true] {
        for (turns, rotation) in (0u8..).zip(ROTATIONS) {
            let [cx, cy] = axes(turns, mirrored);
            let score = ex.0 * cx.0 + ex.1 * cx.1 + ey.0 * cy.0 + ey.1 * cy.1;
            if score > best_score + 1e-9 {
                best_score = score;
                best = (rotation, mirrored);
            }
        }
    }
    best
}

/// Applies `(rotation, mirrored)` to a tightly packed RGBA8 image, returning
/// the displayed pixels and their size.
pub(crate) fn orient_rgba(
    rgba: &[u8],
    width: u32,
    height: u32,
    rotation: Rotation,
    mirrored: bool,
) -> (Vec<u8>, u32, u32) {
    let (w, h) = (width as usize, height as usize);
    let turns = quarter_turns(rotation);
    if (turns == 0 && !mirrored) || rgba.len() < w * h * 4 {
        return (rgba.to_vec(), width, height);
    }
    let (dw, dh) = if turns % 2 == 1 { (h, w) } else { (w, h) };
    let mut out = vec![0u8; dw * dh * 4];
    for y in 0..h {
        for x in 0..w {
            let mx = if mirrored { w - 1 - x } else { x };
            let (tx, ty) = match turns {
                1 => (h - 1 - y, mx),
                2 => (w - 1 - mx, h - 1 - y),
                3 => (y, w - 1 - mx),
                _ => (mx, y),
            };
            let s = (y * w + x) * 4;
            let d = (ty * dw + tx) * 4;
            out[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
        }
    }
    (out, dw as u32, dh as u32)
}

/// The largest size with the aspect ratio of `width × height` whose longer
/// side is at most `max_side` (never upscaled, never below 1 px).
pub(crate) fn fit_within(width: u32, height: u32, max_side: u32) -> (u32, u32) {
    let long = width.max(height);
    if long == 0 || long <= max_side {
        return (width.max(1), height.max(1));
    }
    let scale = |v: u32| {
        let s = (u64::from(v) * u64::from(max_side) + u64::from(long) / 2) / u64::from(long);
        u32::try_from(s).unwrap_or(max_side).max(1)
    };
    (scale(width), scale(height))
}

/// Coded size → square-pixel size: the width is scaled by the sample aspect
/// ratio (rounded to nearest).
pub(crate) fn square_pixel_size(width: u32, height: u32, sar: (u32, u32)) -> (u32, u32) {
    let (n, d) = sar;
    if n == 0 || d == 0 || n == d {
        return (width, height);
    }
    let w = (u64::from(width) * u64::from(n) + u64::from(d) / 2) / u64::from(d);
    (u32::try_from(w).unwrap_or(u32::MAX).max(1), height)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)] // test code
    use super::*;

    /// FFmpeg's `av_display_rotation_set(m, cw_degrees)` for quarter turns
    /// (2^16 fixed point), and `av_display_matrix_flip(m, hflip, vflip)`.
    fn ffmpeg_matrix(cw_degrees: i32, hflip: bool, vflip: bool) -> [i32; 9] {
        let one = 1 << 16;
        let (c, s) = match cw_degrees.rem_euclid(360) {
            0 => (one, 0),
            90 => (0, -one),
            180 => (-one, 0),
            _ => (0, one),
        };
        let mut m = [c, -s, 0, s, c, 0, 0, 0, 1 << 30];
        if hflip {
            for i in [0, 3, 6] {
                m[i] = -m[i];
            }
        }
        if vflip {
            for i in [1, 4, 7] {
                m[i] = -m[i];
            }
        }
        m
    }

    #[test]
    fn pure_rotations() {
        assert_eq!(
            orientation_from_matrix(&ffmpeg_matrix(0, false, false)),
            (Rotation::None, false)
        );
        assert_eq!(
            orientation_from_matrix(&ffmpeg_matrix(90, false, false)),
            (Rotation::Cw90, false)
        );
        assert_eq!(
            orientation_from_matrix(&ffmpeg_matrix(180, false, false)),
            (Rotation::Cw180, false)
        );
        assert_eq!(
            orientation_from_matrix(&ffmpeg_matrix(270, false, false)),
            (Rotation::Cw270, false)
        );
    }

    #[test]
    fn flips() {
        // A horizontal flip of the displayed picture.
        assert_eq!(
            orientation_from_matrix(&ffmpeg_matrix(0, true, false)),
            (Rotation::None, true)
        );
        // A vertical flip is a mirror plus a half turn.
        assert_eq!(
            orientation_from_matrix(&ffmpeg_matrix(0, false, true)),
            (Rotation::Cw180, true)
        );
        // Both flips together are a half turn.
        assert_eq!(
            orientation_from_matrix(&ffmpeg_matrix(0, true, true)),
            (Rotation::Cw180, false)
        );
    }

    #[test]
    fn degenerate_and_skewed_matrices() {
        assert_eq!(orientation_from_matrix(&[0; 9]), (Rotation::None, false));
        // 80° clockwise rounds to a quarter turn.
        let (c, s) = (
            (80f64.to_radians().cos() * 65536.0) as i32,
            (80f64.to_radians().sin() * 65536.0) as i32,
        );
        let m = [c, s, 0, -s, c, 0, 0, 0, 1 << 30];
        assert_eq!(orientation_from_matrix(&m), (Rotation::Cw90, false));
    }

    /// 2×1 image: red, green.
    fn two_pixels() -> Vec<u8> {
        vec![255, 0, 0, 255, 0, 255, 0, 255]
    }

    #[test]
    fn orientation_moves_pixels() {
        let img = two_pixels();
        let (out, w, h) = orient_rgba(&img, 2, 1, Rotation::Cw90, false);
        // Clockwise: the left pixel ends on top.
        assert_eq!((w, h), (1, 2));
        assert_eq!(&out[..4], &[255, 0, 0, 255]);
        let (out, w, h) = orient_rgba(&img, 2, 1, Rotation::Cw270, false);
        assert_eq!((w, h), (1, 2));
        assert_eq!(&out[..4], &[0, 255, 0, 255]);
        let (out, _, _) = orient_rgba(&img, 2, 1, Rotation::None, true);
        assert_eq!(&out[..4], &[0, 255, 0, 255]);
        let (out, _, _) = orient_rgba(&img, 2, 1, Rotation::Cw180, false);
        assert_eq!(&out[..4], &[0, 255, 0, 255]);
        // Mirror then a quarter turn: the (mirrored) left pixel is green.
        let (out, _, _) = orient_rgba(&img, 2, 1, Rotation::Cw90, true);
        assert_eq!(&out[..4], &[0, 255, 0, 255]);
    }

    #[test]
    fn matrices_and_pixels_agree() {
        // For every combination, the matrix-derived orientation moves the
        // coded x axis where the matrix says.
        for deg in [0, 90, 180, 270] {
            for hflip in [false, true] {
                let m = ffmpeg_matrix(deg, hflip, false);
                let (rot, mir) = orientation_from_matrix(&m);
                let [ex, ey] = axes(quarter_turns(rot), mir);
                let round = |(x, y): (f64, f64)| (x.round() as i32, y.round() as i32);
                assert_eq!(round(ex), (m[0].signum(), m[1].signum()), "{deg} {hflip}");
                assert_eq!(round(ey), (m[3].signum(), m[4].signum()), "{deg} {hflip}");
            }
        }
    }

    #[test]
    fn fitting() {
        assert_eq!(fit_within(1920, 1080, 256), (256, 144));
        assert_eq!(fit_within(1080, 1920, 256), (144, 256));
        assert_eq!(fit_within(100, 50, 256), (100, 50));
        assert_eq!(fit_within(10_000, 1, 256), (256, 1));
        assert_eq!(square_pixel_size(1440, 1080, (4, 3)), (1920, 1080));
        assert_eq!(square_pixel_size(720, 480, (0, 1)), (720, 480));
    }
}
