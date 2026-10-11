//! LUT files (CL-016): FreeCut's own parsers for `.cube` (1D, 3D, and a 1D
//! shaper before a 3D table) and `.3dl`, and the sampling the renderer
//! shares (CL-017).
//!
//! Written from Adobe's public Cube LUT 1.0 specification and public `.3dl`
//! descriptions; the validation rules are FreeCut's (color-and-filters.md,
//! *`.cube` and `.3dl` validation*).
//!
//! - [`parse_cube`] and [`parse_3dl`] take the file's bytes and return a
//!   [`Lut`] or a [`LutError`] that names the line and one reason. They are
//!   pure: the caller reads the file on a worker thread, never the UI
//!   thread.
//! - [`Lut`] keeps the tables as `f32` lattices, red varying fastest
//!   (`index = r + n·g + n²·b`); `.3dl` data is reordered on parse.
//! - [`Lut::apply_tetrahedral`] and [`Lut::apply_trilinear`] map one pixel:
//!   clamp to the domain, normalize to 0–1, the 1D table (linear per
//!   channel), then the 3D table. Plain `f32` arithmetic in a fixed order,
//!   so preview and export give identical bytes.
//!
//! This module needs no FFmpeg and is built without the `ffmpeg` feature.

mod cube;
mod error;
mod text;
mod three_dl;

pub use cube::parse_cube;
pub use error::{LutError, LutErrorKind, Malformed, Unsupported};
pub use three_dl::parse_3dl;

/// Files larger than this (600 MiB) are refused before parsing
/// ([`LutErrorKind::TooLarge`]); a 256³ `.cube` is about 500 MB of text.
pub const MAX_FILE_BYTES: usize = 600 * 1024 * 1024;

/// The largest `LUT_1D_SIZE`.
pub const MAX_1D_SIZE: u32 = 65_536;

/// The largest `LUT_3D_SIZE` a `.cube` may declare.
pub const MAX_CUBE_3D_SIZE: u32 = 256;

/// The largest 3D size kept as is; larger `.cube` tables are resampled to
/// [`RESAMPLED_3D_SIZE`] ([`LutNotice::Resampled`]).
pub const MAX_KEPT_3D_SIZE: u32 = 129;

/// The size larger `.cube` tables are resampled to.
pub const RESAMPLED_3D_SIZE: u32 = 65;

/// The most values a `.3dl` input mesh may have (its cube size).
pub const MAX_3DL_MESH: usize = 65;

/// The longest `TITLE`, in characters.
pub const MAX_TITLE_CHARS: usize = 255;

/// How far a table may be from identity and still count as one
/// ([`Lut::is_identity`]): one 16-bit code value.
pub const IDENTITY_TOLERANCE: f32 = 1.0 / 65_535.0;

/// A parsed LUT file.
#[derive(Debug, Clone, PartialEq)]
pub struct Lut {
    /// The `.cube` `TITLE`, the default display name (`None` when absent or
    /// blank, and for `.3dl`).
    pub title: Option<String>,
    /// The 1D table: the whole LUT of a 1D `.cube`, or the shaper applied
    /// before [`Lut::cube`].
    pub shaper: Option<Lut1d>,
    /// The 3D table.
    pub cube: Option<Lut3d>,
    /// The input domain's lower corner (`DOMAIN_MIN`, default 0 0 0).
    pub domain_min: [f32; 3],
    /// The input domain's upper corner (`DOMAIN_MAX`, default 1 1 1); every
    /// channel above `domain_min`.
    pub domain_max: [f32; 3],
    /// Unknown upper-case keywords that were skipped (the import summary
    /// mentions them).
    pub skipped_keywords: u32,
    /// What the parser changed on the way in, for the import summary.
    pub notices: Vec<LutNotice>,
}

/// Something the parser changed while reading a valid file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LutNotice {
    /// A `.cube` 3D table larger than [`MAX_KEPT_3D_SIZE`] was resampled
    /// (trilinear) to [`RESAMPLED_3D_SIZE`].
    Resampled {
        /// The declared size.
        from: u32,
        /// The size kept.
        to: u32,
    },
    /// A `.3dl` input mesh that is not evenly spaced was resampled onto an
    /// even lattice of the same size.
    MeshResampled {
        /// The cube size.
        size: u32,
    },
}

impl Lut {
    /// Clamps `rgb` to the domain and maps it to 0–1 per channel. NaN
    /// becomes the domain minimum.
    pub fn normalize(&self, rgb: [f32; 3]) -> [f32; 3] {
        let mut out = [0.0; 3];
        for c in 0..3 {
            let lo = self.domain_min[c];
            let hi = self.domain_max[c];
            out[c] = (rgb[c].max(lo).min(hi) - lo) / (hi - lo);
        }
        out
    }

    /// Maps one pixel with tetrahedral interpolation in the 3D table (the
    /// default, CL-017).
    pub fn apply_tetrahedral(&self, rgb: [f32; 3]) -> [f32; 3] {
        self.apply(rgb, Lut3d::sample_tetrahedral)
    }

    /// Maps one pixel with trilinear interpolation in the 3D table.
    pub fn apply_trilinear(&self, rgb: [f32; 3]) -> [f32; 3] {
        self.apply(rgb, Lut3d::sample_trilinear)
    }

    fn apply(&self, rgb: [f32; 3], sample3: fn(&Lut3d, [f32; 3]) -> [f32; 3]) -> [f32; 3] {
        let mut v = self.normalize(rgb);
        if let Some(shaper) = &self.shaper {
            v = shaper.sample(v);
        }
        if let Some(cube) = &self.cube {
            v = sample3(cube, v);
        }
        v
    }

    /// Whether the LUT changes nothing: a 0–1 domain and every table entry
    /// within [`IDENTITY_TOLERANCE`] of its own lattice position. The
    /// library entry then says "No visible change" (CL-016).
    pub fn is_identity(&self) -> bool {
        self.domain_min == [0.0; 3]
            && self.domain_max == [1.0; 3]
            && self.shaper.as_ref().is_none_or(Lut1d::is_identity)
            && self.cube.as_ref().is_none_or(Lut3d::is_identity)
    }
}

/// A 1D table: `size` entries, one output per channel, at evenly spaced
/// inputs 0..=1.
#[derive(Debug, Clone, PartialEq)]
pub struct Lut1d {
    data: Vec<[f32; 3]>,
}

impl Lut1d {
    /// A table from its entries; `None` unless there are
    /// 2..=[`MAX_1D_SIZE`] of them.
    pub fn new(data: Vec<[f32; 3]>) -> Option<Lut1d> {
        (2..=MAX_1D_SIZE as usize)
            .contains(&data.len())
            .then_some(Lut1d { data })
    }

    /// The number of entries (2..=[`MAX_1D_SIZE`]).
    pub fn size(&self) -> u32 {
        // The length is checked against `MAX_1D_SIZE` in `new`.
        u32::try_from(self.data.len()).unwrap_or(MAX_1D_SIZE)
    }

    /// The entries, input 0 first.
    pub fn data(&self) -> &[[f32; 3]] {
        &self.data
    }

    /// Looks up each channel separately with linear interpolation; the
    /// input is clamped to 0–1.
    pub fn sample(&self, rgb: [f32; 3]) -> [f32; 3] {
        let mut out = [0.0; 3];
        for c in 0..3 {
            let (i, t) = cell(rgb[c], self.data.len());
            let a = self.data[i][c];
            let b = self.data[i + 1][c];
            out[c] = a + (b - a) * t;
        }
        out
    }

    fn is_identity(&self) -> bool {
        let last = (self.data.len() - 1) as f32;
        self.data.iter().enumerate().all(|(i, v)| {
            let x = i as f32 / last;
            v.iter().all(|&o| (o - x).abs() <= IDENTITY_TOLERANCE)
        })
    }
}

/// A 3D table: `size³` entries at an even lattice over 0..=1 per channel,
/// red varying fastest (`index = r + n·g + n²·b`).
#[derive(Debug, Clone, PartialEq)]
pub struct Lut3d {
    size: usize,
    data: Vec<[f32; 3]>,
}

impl Lut3d {
    /// A table from its size and entries; `None` unless `size` is
    /// 2..=[`MAX_CUBE_3D_SIZE`] and there are `size³` entries.
    pub fn new(size: u32, data: Vec<[f32; 3]>) -> Option<Lut3d> {
        let n = size as usize;
        ((2..=MAX_CUBE_3D_SIZE).contains(&size) && data.len() == n * n * n)
            .then_some(Lut3d { size: n, data })
    }

    /// A table of `size³` entries computed from each lattice point's input
    /// (0..=1 per channel); `None` unless `size` is 2..=[`MAX_CUBE_3D_SIZE`].
    /// FreeCut's own bundled looks are generated this way.
    pub fn from_fn(size: u32, mut f: impl FnMut([f32; 3]) -> [f32; 3]) -> Option<Lut3d> {
        if !(2..=MAX_CUBE_3D_SIZE).contains(&size) {
            return None;
        }
        let n = size as usize;
        let last = (n - 1) as f32;
        let mut data = Vec::with_capacity(n * n * n);
        for b in 0..n {
            for g in 0..n {
                for r in 0..n {
                    data.push(f([r as f32 / last, g as f32 / last, b as f32 / last]));
                }
            }
        }
        Some(Lut3d { size: n, data })
    }

    /// The lattice size `n` (2..=[`MAX_CUBE_3D_SIZE`]).
    pub fn size(&self) -> u32 {
        // `new` and `from_fn` bound the size by `MAX_CUBE_3D_SIZE`.
        u32::try_from(self.size).unwrap_or(MAX_CUBE_3D_SIZE)
    }

    /// The entries, red fastest.
    pub fn data(&self) -> &[[f32; 3]] {
        &self.data
    }

    /// The entry at lattice indices `r`, `g`, `b` (each `< size`).
    ///
    /// # Panics
    /// When an index is out of range.
    pub fn at(&self, r: usize, g: usize, b: usize) -> [f32; 3] {
        assert!(r < self.size && g < self.size && b < self.size);
        self.data[r + self.size * (g + self.size * b)]
    }

    /// Trilinear interpolation; the input is clamped to 0–1.
    pub fn sample_trilinear(&self, rgb: [f32; 3]) -> [f32; 3] {
        let (ri, rt) = cell(rgb[0], self.size);
        let (gi, gt) = cell(rgb[1], self.size);
        let (bi, bt) = cell(rgb[2], self.size);
        let at = |r: usize, g: usize, b: usize| self.at(ri + r, gi + g, bi + b);
        trilinear(
            [
                [at(0, 0, 0), at(1, 0, 0)],
                [at(0, 1, 0), at(1, 1, 0)],
                [at(0, 0, 1), at(1, 0, 1)],
                [at(0, 1, 1), at(1, 1, 1)],
            ],
            [rt, gt, bt],
        )
    }

    /// Tetrahedral interpolation (the cell is split into six tetrahedra by
    /// the order of the fractional parts); the input is clamped to 0–1.
    pub fn sample_tetrahedral(&self, rgb: [f32; 3]) -> [f32; 3] {
        let (ri, fr) = cell(rgb[0], self.size);
        let (gi, fg) = cell(rgb[1], self.size);
        let (bi, fb) = cell(rgb[2], self.size);
        let at = |r: usize, g: usize, b: usize| self.at(ri + r, gi + g, bi + b);
        let c000 = at(0, 0, 0);
        let c111 = at(1, 1, 1);
        // (weights of c000, first corner, second corner, c111)
        let (w0, p, w1, q, w2, w3) = if fr > fg {
            if fg > fb {
                (1.0 - fr, at(1, 0, 0), fr - fg, at(1, 1, 0), fg - fb, fb)
            } else if fr > fb {
                (1.0 - fr, at(1, 0, 0), fr - fb, at(1, 0, 1), fb - fg, fg)
            } else {
                (1.0 - fb, at(0, 0, 1), fb - fr, at(1, 0, 1), fr - fg, fg)
            }
        } else if fb > fg {
            (1.0 - fb, at(0, 0, 1), fb - fg, at(0, 1, 1), fg - fr, fr)
        } else if fb > fr {
            (1.0 - fg, at(0, 1, 0), fg - fb, at(0, 1, 1), fb - fr, fr)
        } else {
            (1.0 - fg, at(0, 1, 0), fg - fr, at(1, 1, 0), fr - fb, fb)
        };
        std::array::from_fn(|c| w0 * c000[c] + w1 * p[c] + w2 * q[c] + w3 * c111[c])
    }

    /// This table resampled (trilinear) onto an even lattice of `size`.
    pub(crate) fn resampled(&self, size: u32) -> Option<Lut3d> {
        Lut3d::from_fn(size, |rgb| self.sample_trilinear(rgb))
    }

    fn is_identity(&self) -> bool {
        let n = self.size;
        let last = (n - 1) as f32;
        self.data.iter().enumerate().all(|(i, v)| {
            let input = [
                (i % n) as f32 / last,
                (i / n % n) as f32 / last,
                (i / (n * n)) as f32 / last,
            ];
            (0..3).all(|c| (v[c] - input[c]).abs() <= IDENTITY_TOLERANCE)
        })
    }
}

/// The lattice cell holding `x` (clamped to 0–1, NaN as 0) in a table of
/// `len ≥ 2` evenly spaced entries: the lower index (`< len − 1`) and the
/// fraction 0..=1 toward the next.
fn cell(x: f32, len: usize) -> (usize, f32) {
    let last = len - 1;
    // Not `clamp`: NaN must land on 0, and `clamp` keeps it.
    #[allow(clippy::manual_clamp)]
    let pos = x.max(0.0).min(1.0) * last as f32;
    // `pos` is finite and in 0..=last, so the cast is exact on the floor.
    let i = (pos.floor() as usize).min(last - 1);
    (i, pos - i as f32)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Trilinear blend of a cell's corners, given as `[g0b0, g1b0, g0b1, g1b1]`
/// pairs of `[r0, r1]`, at fractions `[r, g, b]`: red first, then green,
/// then blue (a fixed order, so results are reproducible).
pub(crate) fn trilinear(corners: [[[f32; 3]; 2]; 4], [rt, gt, bt]: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|c| {
        let [x00, x10, x01, x11] = corners.map(|[lo, hi]| lerp(lo[c], hi[c], rt));
        lerp(lerp(x00, x10, gt), lerp(x01, x11, gt), bt)
    })
}

/// Refuses files over [`MAX_FILE_BYTES`] before parsing.
fn check_len(len: usize) -> Result<(), LutError> {
    if len > MAX_FILE_BYTES {
        Err(LutError {
            line: None,
            kind: LutErrorKind::TooLarge,
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(n: u32) -> Lut3d {
        Lut3d::from_fn(n, |rgb| rgb).unwrap()
    }

    fn close(a: [f32; 3], b: [f32; 3], tol: f32) -> bool {
        (0..3).all(|c| (a[c] - b[c]).abs() <= tol)
    }

    #[test]
    fn too_large_files_are_refused_before_parsing() {
        assert!(check_len(MAX_FILE_BYTES).is_ok());
        let e = check_len(MAX_FILE_BYTES + 1).unwrap_err();
        assert_eq!(e.line, None);
        assert_eq!(e.kind, LutErrorKind::TooLarge);
        assert_eq!(
            e.to_string(),
            "the file is larger than 600 MB, the most FreeCut reads as a LUT"
        );
    }

    #[test]
    fn constructors_check_sizes() {
        assert!(Lut1d::new(vec![[0.0; 3]]).is_none());
        assert!(Lut1d::new(vec![[0.0; 3]; 2]).is_some());
        assert!(Lut3d::new(2, vec![[0.0; 3]; 7]).is_none());
        assert!(Lut3d::new(1, vec![[0.0; 3]; 1]).is_none());
        assert_eq!(Lut3d::new(2, vec![[0.0; 3]; 8]).unwrap().size(), 2);
        assert!(Lut3d::from_fn(257, |v| v).is_none());
    }

    #[test]
    fn red_varies_fastest() {
        let t = Lut3d::from_fn(3, |rgb| rgb).unwrap();
        assert_eq!(t.data()[1], [0.5, 0.0, 0.0]);
        assert_eq!(t.data()[3], [0.0, 0.5, 0.0]);
        assert_eq!(t.data()[9], [0.0, 0.0, 0.5]);
        assert_eq!(t.at(2, 1, 0), [1.0, 0.5, 0.0]);
    }

    #[test]
    fn identity_tables_reproduce_their_input() {
        let t = identity(17);
        for &rgb in &[
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0],
            [0.3, 0.7, 0.1],
            [0.99, 0.01, 0.5],
            [0.5, 0.5, 0.5],
        ] {
            assert!(close(t.sample_trilinear(rgb), rgb, 1e-6), "{rgb:?}");
            assert!(close(t.sample_tetrahedral(rgb), rgb, 1e-6), "{rgb:?}");
        }
    }

    #[test]
    fn lattice_points_are_exact() {
        // A non-linear table: interpolation must return the entry itself at
        // every lattice point, in all six tetrahedra and at the top edge.
        let t = Lut3d::from_fn(5, |[r, g, b]| [r * r, g.sqrt(), 1.0 - b * g]).unwrap();
        for b in 0..5 {
            for g in 0..5 {
                for r in 0..5 {
                    let x = [r as f32 / 4.0, g as f32 / 4.0, b as f32 / 4.0];
                    assert_eq!(t.sample_tetrahedral(x), t.at(r, g, b));
                    assert_eq!(t.sample_trilinear(x), t.at(r, g, b));
                }
            }
        }
    }

    #[test]
    fn tetrahedral_is_continuous_across_tetrahedra() {
        let t = Lut3d::from_fn(2, |[r, g, b]| [r * g, g * b + r, (r + b) * 0.5]).unwrap();
        // Points on the diagonal planes where two tetrahedra meet.
        for &rgb in &[[0.4, 0.4, 0.2], [0.3, 0.6, 0.6], [0.7, 0.2, 0.7]] {
            let eps = 1e-4;
            let a = t.sample_tetrahedral(rgb);
            for d in 0..3 {
                let mut p = rgb;
                p[d] += eps;
                assert!(close(a, t.sample_tetrahedral(p), 1e-3), "{rgb:?} {d}");
            }
        }
    }

    #[test]
    fn inputs_are_clamped_and_nan_is_safe() {
        let t = identity(3);
        assert_eq!(t.sample_tetrahedral([-1.0, 2.0, 0.5]), [0.0, 1.0, 0.5]);
        assert_eq!(t.sample_trilinear([f32::NAN, 0.0, 1.0]), [0.0, 0.0, 1.0]);
        let one = Lut1d::new(vec![[0.0, 1.0, 0.5], [1.0, 0.0, 0.5]]).unwrap();
        assert_eq!(one.sample([0.25, 0.25, f32::NAN]), [0.25, 0.75, 0.5]);
    }

    #[test]
    fn domain_is_clamped_then_normalized() {
        let lut = Lut {
            title: None,
            shaper: None,
            cube: Some(identity(2)),
            domain_min: [0.0, -1.0, 0.0],
            domain_max: [2.0, 1.0, 1.0],
            skipped_keywords: 0,
            notices: vec![],
        };
        assert!(close(
            lut.apply_tetrahedral([1.0, 0.0, 3.0]),
            [0.5, 0.5, 1.0],
            1e-6
        ));
        assert!(!lut.is_identity());
    }

    #[test]
    fn shaper_runs_before_the_cube() {
        let lut = Lut {
            title: None,
            shaper: Lut1d::new(vec![[0.0; 3], [0.25; 3], [1.0; 3]]),
            cube: Lut3d::from_fn(2, |[r, g, b]| [1.0 - r, 1.0 - g, 1.0 - b]),
            domain_min: [0.0; 3],
            domain_max: [1.0; 3],
            skipped_keywords: 0,
            notices: vec![],
        };
        assert!(close(
            lut.apply_trilinear([0.5, 0.0, 1.0]),
            [0.75, 1.0, 0.0],
            1e-6
        ));
    }

    #[test]
    fn identity_detection_uses_one_16_bit_step() {
        let mut lut = Lut {
            title: None,
            shaper: None,
            cube: Some(identity(3)),
            domain_min: [0.0; 3],
            domain_max: [1.0; 3],
            skipped_keywords: 0,
            notices: vec![],
        };
        assert!(lut.is_identity());
        let mut data = identity(3).data().to_vec();
        data[4][1] += 0.5 / 65_535.0;
        lut.cube = Lut3d::new(3, data.clone());
        assert!(lut.is_identity());
        data[4][1] += 2.0 / 65_535.0;
        lut.cube = Lut3d::new(3, data);
        assert!(!lut.is_identity());
    }

    #[test]
    fn resampling_keeps_a_linear_table() {
        let src = Lut3d::from_fn(9, |[r, g, b]| [0.5 * r + 0.1, g, 1.0 - b]).unwrap();
        let dst = src.resampled(5).unwrap();
        assert_eq!(dst.size(), 5);
        let back = Lut3d::from_fn(5, |[r, g, b]| [0.5 * r + 0.1, g, 1.0 - b]).unwrap();
        for (a, b) in dst.data().iter().zip(back.data()) {
            assert!(close(*a, *b, 1e-6));
        }
    }

    #[test]
    fn sampling_is_deterministic() {
        let t = Lut3d::from_fn(7, |[r, g, b]| [r * g, (g + b) * 0.5, r.max(b)]).unwrap();
        let x = [0.123, 0.456, 0.789];
        let a = t.sample_tetrahedral(x).map(f32::to_bits);
        for _ in 0..4 {
            assert_eq!(t.sample_tetrahedral(x).map(f32::to_bits), a);
        }
    }
}
