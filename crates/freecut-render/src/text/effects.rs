//! Mask effects in layer space: an exact Euclidean distance transform of
//! coverage masks, the signed distance field built on it, the bands and
//! sources derived from that field (outline, borders), and a separable
//! Gaussian blur for shadows and glows.
//!
//! Masks are `f32` coverage, 0..=1, one value per pixel, rows top to
//! bottom. Everything here is plain sequential arithmetic, so results are
//! identical on every thread and every run.

/// "No feature" in the distance transform: far beyond any image (squares
/// of distances in an 8192 px image are below 2^27).
const FAR: f64 = 1e20;

/// The squared Euclidean distance from every pixel centre to the nearest
/// feature pixel centre, and the index of that feature (Felzenszwalb and
/// Huttenlocher's lower-envelope transform, applied to columns and then
/// rows). Without any feature every distance is at least `1e19` and the
/// indices are meaningless.
pub(crate) fn edt(features: &[bool], width: usize, height: usize) -> (Vec<f64>, Vec<u32>) {
    let n = width * height;
    let mut column = vec![0.0f64; n];
    let mut row_of = vec![0u32; n];
    let longest = width.max(height);
    let mut f = vec![0.0f64; longest];
    let mut d = vec![0.0f64; longest];
    let mut arg = vec![0usize; longest];
    let mut v = vec![0usize; longest];
    let mut z = vec![0.0f64; longest + 1];
    // Columns: distance to the nearest feature in the same column.
    for x in 0..width {
        for y in 0..height {
            f[y] = if features[y * width + x] { 0.0 } else { FAR };
        }
        lower_envelope(
            &f[..height],
            &mut d[..height],
            &mut arg[..height],
            &mut v,
            &mut z,
        );
        for y in 0..height {
            column[y * width + x] = d[y];
            row_of[y * width + x] = arg[y] as u32;
        }
    }
    // Rows: combine the column distances.
    let mut dist = vec![0.0f64; n];
    let mut nearest = vec![0u32; n];
    for y in 0..height {
        let row = &column[y * width..(y + 1) * width];
        f[..width].copy_from_slice(row);
        lower_envelope(
            &f[..width],
            &mut d[..width],
            &mut arg[..width],
            &mut v,
            &mut z,
        );
        for x in 0..width {
            let fx = arg[x];
            dist[y * width + x] = d[x];
            let fy = row_of[y * width + fx] as usize;
            nearest[y * width + x] = (fy * width + fx) as u32;
        }
    }
    (dist, nearest)
}

/// The 1-D squared distance transform of the sampled function `f`:
/// `d[q] = min_p (q − p)² + f[p]`, with the minimizing `p` in `arg[q]`.
fn lower_envelope(f: &[f64], d: &mut [f64], arg: &mut [usize], v: &mut [usize], z: &mut [f64]) {
    let n = f.len();
    if n == 0 {
        return;
    }
    let mut k = 0usize;
    v[0] = 0;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    for q in 1..n {
        let fq = f[q] + (q * q) as f64;
        loop {
            let p = v[k];
            let s = (fq - (f[p] + (p * p) as f64)) / (2.0 * (q as f64 - p as f64));
            if s <= z[k] && k > 0 {
                k -= 1;
                continue;
            }
            if s <= z[k] {
                // k == 0: the new parabola is lowest everywhere so far.
                v[0] = q;
                z[0] = f64::NEG_INFINITY;
                z[1] = f64::INFINITY;
                break;
            }
            k += 1;
            v[k] = q;
            z[k] = s;
            z[k + 1] = f64::INFINITY;
            break;
        }
    }
    k = 0;
    for q in 0..n {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let p = v[k];
        let dq = q as f64 - p as f64;
        d[q] = dq * dq + f[p];
        arg[q] = p;
    }
}

/// Signed distance (px, positive outside) from each pixel centre to the
/// edge of the shape whose coverage is `cov`.
///
/// Partly covered pixels sit on the edge: a pixel half covered has its
/// centre on it, so its distance is `0.5 − c`. Other pixels add the exact
/// Euclidean distance to the nearest pixel across the edge (from
/// [`edt`]) to that pixel's own distance, so bands stay antialiased at any
/// width. Uncovered pixels with no shape at all get `+1e6`, full pixels
/// with no edge `−1e6`.
pub(crate) fn signed_distance(cov: &[f32], width: usize, height: usize) -> Vec<f32> {
    let outside_features: Vec<bool> = cov.iter().map(|&c| c > 0.0).collect();
    let inside_features: Vec<bool> = cov.iter().map(|&c| c < 1.0).collect();
    let any_ink = outside_features.iter().any(|&f| f);
    let any_hole = inside_features.iter().any(|&f| f);
    let outside = any_ink.then(|| edt(&outside_features, width, height));
    let inside = any_hole.then(|| edt(&inside_features, width, height));
    let mut sd = vec![0.0f32; cov.len()];
    for (i, (&c, out)) in cov.iter().zip(sd.iter_mut()).enumerate() {
        *out = if c > 0.0 && c < 1.0 {
            0.5 - c
        } else if c <= 0.0 {
            match &outside {
                Some((d2, nearest)) => {
                    let q = nearest[i] as usize;
                    d2[i].sqrt() as f32 + (0.5 - cov[q])
                }
                None => 1e6,
            }
        } else {
            match &inside {
                Some((d2, nearest)) => {
                    let q = nearest[i] as usize;
                    (0.5 - cov[q]) - d2[i].sqrt() as f32
                }
                None => -1e6,
            }
        };
    }
    sd
}

/// Coverage of the shape grown by `r` px (`r < 0` shrinks it).
#[inline]
pub(crate) fn grown(sd: f32, r: f32) -> f32 {
    (r + 0.5 - sd).clamp(0.0, 1.0)
}

/// Kernel radius (px) above which blurs run at a power-of-two reduced
/// resolution.
const DOWNSAMPLE_ABOVE: f32 = 32.0;

/// Largest blur radius in device px (TX-011).
pub(crate) const MAX_BLUR_RADIUS: f32 = 256.0;

/// The standard deviation of the Gaussian for a blur of `radius` px: the
/// visible extent of a blurred edge is about the radius.
pub(crate) fn sigma_for(radius: f32) -> f32 {
    radius.clamp(0.0, MAX_BLUR_RADIUS) / 2.0
}

/// How far (px) a blur of `radius` spreads: three standard deviations.
pub(crate) fn blur_reach(radius: f32) -> f32 {
    (3.0 * sigma_for(radius)).ceil()
}

/// `src` blurred with a Gaussian for `radius` px (capped at
/// [`MAX_BLUR_RADIUS`]); pixels outside the image read as `border`.
/// Radii above 32 px are blurred at the power-of-two reduced resolution
/// that brings them to 32 px or less, then scaled back up bilinearly.
pub(crate) fn blur(src: &[f32], width: usize, height: usize, radius: f32, border: f32) -> Vec<f32> {
    blur_reduced(src, width, height, radius, border, DOWNSAMPLE_ABOVE)
}

/// [`blur`] with the reduced-resolution threshold `downsample_above`
/// (px): radii above it are blurred at the power-of-two reduction that
/// brings them to it or below. Whole-layer blurs (FX-030) use a lower
/// threshold than text effects, trading a little accuracy for speed.
pub(crate) fn blur_reduced(
    src: &[f32],
    width: usize,
    height: usize,
    radius: f32,
    border: f32,
    downsample_above: f32,
) -> Vec<f32> {
    let downsample_above = if downsample_above.is_finite() {
        downsample_above.max(1.0)
    } else {
        DOWNSAMPLE_ABOVE
    };
    let radius = if radius.is_finite() {
        radius.clamp(0.0, MAX_BLUR_RADIUS)
    } else {
        0.0
    };
    let sigma = sigma_for(radius);
    if sigma < 0.2 || width == 0 || height == 0 {
        return src.to_vec();
    }
    let mut factor = 1usize;
    while radius / factor as f32 > downsample_above {
        factor *= 2;
    }
    if factor == 1 {
        return gaussian(src, width, height, sigma, border);
    }
    let (sw, sh) = (width.div_ceil(factor), height.div_ceil(factor));
    let small = downsample(src, width, height, factor, border);
    let blurred = gaussian(&small, sw, sh, sigma / factor as f32, border);
    upsample(&blurred, sw, sh, width, height, factor, border)
}

fn kernel(sigma: f32) -> Vec<f32> {
    let reach = (3.0 * sigma).ceil() as i64;
    let mut k: Vec<f32> = (-reach..=reach)
        .map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp())
        .collect();
    let sum: f32 = k.iter().sum();
    for v in &mut k {
        *v /= sum;
    }
    k
}

/// The separable Gaussian. Every output is the sum of its taps in kernel
/// order, `acc += v · k[j]` from `j = 0`, whatever the loop shape, so the
/// result is the same bit for bit however the passes are laid out. Both
/// passes accumulate whole rows tap by tap (row-major and independent per
/// pixel, so the compiler vectorizes them).
fn gaussian(src: &[f32], width: usize, height: usize, sigma: f32, border: f32) -> Vec<f32> {
    let k = kernel(sigma);
    let reach = k.len() / 2;
    let (w, h) = (width, height);
    let mut tmp = vec![0.0f32; src.len()];
    for y in 0..h {
        let row = &src[y * w..(y + 1) * w];
        let acc = &mut tmp[y * w..(y + 1) * w];
        for (j, &kv) in k.iter().enumerate() {
            // Output x reads source x + j − reach: inside the row for
            // x in lo..hi.
            let lo = reach.saturating_sub(j).min(w);
            let hi = (w + reach).saturating_sub(j).min(w).max(lo);
            let edge = border * kv;
            for a in &mut acc[..lo] {
                *a += edge;
            }
            if hi > lo {
                // lo = reach − j (or 0 when j > reach) here.
                let from = lo + j - reach;
                for (a, &v) in acc[lo..hi].iter_mut().zip(&row[from..from + (hi - lo)]) {
                    *a += v * kv;
                }
            }
            for a in &mut acc[hi..] {
                *a += edge;
            }
        }
    }
    let mut out = vec![0.0f32; src.len()];
    for y in 0..h {
        let acc = &mut out[y * w..(y + 1) * w];
        for (j, &kv) in k.iter().enumerate() {
            let sy = y as i64 + j as i64 - reach as i64;
            if sy < 0 || sy >= h as i64 {
                let edge = border * kv;
                for a in acc.iter_mut() {
                    *a += edge;
                }
            } else {
                let sy = sy as usize;
                for (a, &v) in acc.iter_mut().zip(&tmp[sy * w..(sy + 1) * w]) {
                    *a += v * kv;
                }
            }
        }
    }
    out
}

/// Box-filtered reduction by `factor` (sizes rounded up; missing pixels
/// read as `border`).
fn downsample(src: &[f32], width: usize, height: usize, factor: usize, border: f32) -> Vec<f32> {
    let (sw, sh) = (width.div_ceil(factor), height.div_ceil(factor));
    let mut out = vec![0.0f32; sw * sh];
    let area = (factor * factor) as f32;
    for sy in 0..sh {
        for sx in 0..sw {
            let mut acc = 0.0f32;
            for dy in 0..factor {
                let y = sy * factor + dy;
                for dx in 0..factor {
                    let x = sx * factor + dx;
                    acc += if x < width && y < height {
                        src[y * width + x]
                    } else {
                        border
                    };
                }
            }
            out[sy * sw + sx] = acc / area;
        }
    }
    out
}

/// Bilinear enlargement by `factor`, edges clamped (the reduced image's
/// edge pixels already include the border).
fn upsample(
    small: &[f32],
    sw: usize,
    sh: usize,
    width: usize,
    height: usize,
    factor: usize,
    border: f32,
) -> Vec<f32> {
    let f = factor as f32;
    let (max_x, max_y) = ((sw - 1) as f32, (sh - 1) as f32);
    let mut out = vec![0.0f32; width * height];
    for y in 0..height {
        let sy = ((y as f32 + 0.5) / f - 0.5).clamp(0.0, max_y);
        for x in 0..width {
            let sx = ((x as f32 + 0.5) / f - 0.5).clamp(0.0, max_x);
            out[y * width + x] = bilinear(small, sw, sh, sx, sy, border);
        }
    }
    out
}

/// Bilinear sample at pixel coordinates (centres at integers); outside
/// reads as `border`.
fn bilinear(src: &[f32], w: usize, h: usize, x: f32, y: f32, border: f32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (x0, y0) = (x0 as i64, y0 as i64);
    let at = |xi: i64, yi: i64| -> f32 {
        if xi < 0 || yi < 0 || xi >= w as i64 || yi >= h as i64 {
            border
        } else {
            src[yi as usize * w + xi as usize]
        }
    };
    let top = at(x0, y0) * (1.0 - fx) + at(x0 + 1, y0) * fx;
    let bottom = at(x0, y0 + 1) * (1.0 - fx) + at(x0 + 1, y0 + 1) * fx;
    top * (1.0 - fy) + bottom * fy
}

/// `src` moved by (dx, dy) px, bilinearly; uncovered pixels read 0.
pub(crate) fn shifted(src: &[f32], width: usize, height: usize, dx: f32, dy: f32) -> Vec<f32> {
    if dx == 0.0 && dy == 0.0 {
        return src.to_vec();
    }
    let mut out = vec![0.0f32; src.len()];
    for y in 0..height {
        for x in 0..width {
            out[y * width + x] = bilinear(src, width, height, x as f32 - dx, y as f32 - dy, 0.0);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small deterministic generator for test masks.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            self.0 >> 33
        }
    }

    fn brute_force(features: &[bool], w: usize, h: usize) -> Vec<f64> {
        (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as i64, (i / w) as i64);
                features
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| **f)
                    .map(|(j, _)| {
                        let (fx, fy) = ((j % w) as i64, (j / w) as i64);
                        ((x - fx).pow(2) + (y - fy).pow(2)) as f64
                    })
                    .fold(f64::INFINITY, f64::min)
            })
            .collect()
    }

    #[test]
    fn distance_transform_matches_brute_force_on_random_masks() {
        let mut rng = Lcg(12345);
        for case in 0..60 {
            let w = 1 + (rng.next() % 37) as usize;
            let h = 1 + (rng.next() % 29) as usize;
            // Densities from very sparse to nearly full.
            let density = 1 + rng.next() % 60;
            let features: Vec<bool> = (0..w * h).map(|_| rng.next() % 100 < density).collect();
            let (got, nearest) = edt(&features, w, h);
            if !features.iter().any(|f| *f) {
                assert!(got.iter().all(|&d| d >= 1e19), "case {case}");
                continue;
            }
            let want = brute_force(&features, w, h);
            assert_eq!(got, want, "case {case}: {w}×{h}");
            // The reported nearest feature is a feature at that distance.
            for (i, &q) in nearest.iter().enumerate() {
                let q = q as usize;
                assert!(features[q], "case {case}: pixel {i}");
                let (dx, dy) = (
                    (i % w) as f64 - (q % w) as f64,
                    (i / w) as f64 - (q / w) as f64,
                );
                assert_eq!(dx * dx + dy * dy, got[i], "case {case}: pixel {i}");
            }
        }
    }

    /// A `w × h` mask with a solid rectangle of whole pixels.
    fn rect_mask(w: usize, h: usize, x0: usize, y0: usize, x1: usize, y1: usize) -> Vec<f32> {
        (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                if (x0..x1).contains(&x) && (y0..y1).contains(&y) {
                    1.0
                } else {
                    0.0
                }
            })
            .collect()
    }

    #[test]
    fn signed_distance_of_a_square() {
        let (w, h) = (40, 40);
        let cov = rect_mask(w, h, 10, 10, 30, 30);
        let sd = signed_distance(&cov, w, h);
        // The row through the middle: pixel 9 is half a pixel outside the
        // left edge, pixel 10 half a pixel inside.
        let at = |x: usize| sd[20 * w + x];
        assert!((at(9) - 0.5).abs() < 1e-6, "{}", at(9));
        assert!((at(10) + 0.5).abs() < 1e-6, "{}", at(10));
        assert!((at(4) - 5.5).abs() < 1e-6);
        assert!((at(15) + 5.5).abs() < 1e-6);
        // Growing by r covers exactly r more pixels on each side.
        let grown_cols: f32 = (0..w).map(|x| grown(at(x), 5.0)).sum();
        assert!((grown_cols - 30.0).abs() < 1e-4, "{grown_cols}");
        let shrunk: f32 = (0..w).map(|x| grown(at(x), -3.0)).sum();
        assert!((shrunk - 14.0).abs() < 1e-4, "{shrunk}");
    }

    #[test]
    fn partial_edges_give_sub_pixel_distances() {
        // A vertical edge at x = 10.25: pixel 10 is 25 % covered.
        let (w, h) = (24, 3);
        let cov: Vec<f32> = (0..w * h)
            .map(|i| match i % w {
                x if x < 10 => 1.0,
                10 => 0.25,
                _ => 0.0,
            })
            .collect();
        let sd = signed_distance(&cov, w, h);
        for x in 0..w {
            let centre = x as f32 + 0.5;
            let exact = centre - 10.25;
            assert!(
                (sd[w + x] - exact).abs() < 1e-5,
                "x {x}: {} vs {exact}",
                sd[w + x]
            );
        }
    }

    #[test]
    fn blur_preserves_mass_and_spreads() {
        let (w, h) = (64, 64);
        let cov = rect_mask(w, h, 28, 28, 36, 36);
        let b = blur(&cov, w, h, 8.0, 0.0);
        let mass: f32 = b.iter().sum();
        assert!((mass - 64.0).abs() < 0.01, "{mass}");
        assert!(b[10 * w + 32] < 1e-3);
        assert!(b[24 * w + 32] > 0.01);
        // Symmetric around the square's centre.
        assert!((b[32 * w + 24] - b[32 * w + 39]).abs() < 1e-5);
    }

    #[test]
    fn large_blurs_are_downsampled_and_stay_close() {
        let (w, h) = (256, 64);
        let cov = rect_mask(w, h, 96, 0, 160, 64);
        // Infinite vertical extent through the border value.
        let full = gaussian(&cov, w, h, sigma_for(40.0), 0.0);
        let fast = blur(&cov, w, h, 40.0, 0.0);
        let worst = full
            .iter()
            .zip(&fast)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);

        assert!(worst < 0.03, "{worst}");
    }

    #[test]
    fn shifting_moves_whole_pixels_exactly() {
        let (w, h) = (8, 8);
        let cov = rect_mask(w, h, 1, 1, 3, 3);
        let s = shifted(&cov, w, h, 2.0, 3.0);
        assert_eq!(s, rect_mask(w, h, 3, 4, 5, 6));
    }
}
