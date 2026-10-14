//! The colour stack on the CPU path (CL-001, README C52,
//! render-compositor.md §17): the reference evaluator, the baked lattice,
//! grading pictures and the caches that keep a still or paused frame from
//! being graded twice.
//!
//! - [`eval_pixel`] / [`Evaluator`] (`eval`): every per-pixel stage in
//!   CL-001's order, in `f32` with `libm`; the oracle of everything else.
//! - [`bake`] (`bake`): the per-pixel run baked into a 33³ [`Lattice`] over
//!   the 8-bit input cube, applied with tetrahedral interpolation.
//! - [`grade`]: a picture through a [`CompiledStack`], quantized once with
//!   `round(clamp(v, 0, 1)·255)`; alpha and fully transparent pixels are
//!   never changed.
//! - [`ColorDigest`]: what a compiled stack and a graded picture are cached
//!   by.
//!
//! The stack arrives evaluated (keyed values already worked out by the
//! timeline) on `LayerPlan::color`; `None` there means nothing to do.

mod bake;
mod eval;

use std::sync::Arc;

use freecut_timeline::model::{ColorStack, Section};

use crate::cpu::Picture;

pub use bake::{BakeSize, Lattice, bake, bake_with};
pub use eval::{Evaluator, LUMA, eval_pixel, linear_to_ws, luma, ws_to_linear};

/// The version of the colour math. Bump it whenever any stage's arithmetic
/// changes, so caches keyed by [`ColorDigest`] never mix two versions (like
/// the renderer's ABI version).
pub const COLOR_ALGO_VERSION: u32 = 1;

/// XXH3-128 of a canonical encoding of what a stack does: the algorithm
/// version, the bake size, Amount, and each section's switch and (only
/// while it is on) its values. Two stacks that grade alike share a digest;
/// a switched-off section's values don't change it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColorDigest(pub u128);

impl ColorDigest {
    pub fn of(stack: &ColorStack, size: BakeSize) -> ColorDigest {
        let mut bytes = Vec::with_capacity(64);
        bytes.extend_from_slice(b"freecut-color");
        bytes.extend_from_slice(&COLOR_ALGO_VERSION.to_le_bytes());
        bytes.push(size.nodes() as u8);
        bytes.push(u8::from(stack.bypass));
        bytes.push(stack.amount);
        for section in [Section::Light, Section::Color] {
            let on = stack.sections_enabled.is_on(section);
            bytes.push(u8::from(on));
            if on {
                for id in section.params() {
                    bytes.extend_from_slice(&stack.get(*id).to_le_bytes());
                }
            }
        }
        ColorDigest(xxhash_rust::xxh3::xxh3_128(&bytes))
    }
}

/// A stack ready to grade with: its lattice and its digest.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledStack {
    pub digest: ColorDigest,
    pub lattice: Arc<Lattice>,
}

impl CompiledStack {
    /// Bakes `stack` (on up to 8 threads).
    pub fn new(stack: &ColorStack) -> CompiledStack {
        let size = BakeSize::for_stack(stack);
        CompiledStack {
            digest: ColorDigest::of(stack, size),
            lattice: bake(stack, size),
        }
    }
}

/// One 8-bit channel from a stack value: `round(clamp(v, 0, 1)·255)`.
#[inline]
pub fn quantize(v: f32) -> u8 {
    // In 0.5..=255.5 after the clamp, so the truncating cast rounds.
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// `picture` graded by `stack` on `threads` threads (at least one). Rows
/// are split into bands and every pixel is computed on its own, so the
/// bytes don't depend on the thread count. Alpha is kept, and pixels with
/// alpha 0 keep their colour too (CL-001).
pub fn grade(stack: &CompiledStack, picture: &Picture, threads: usize) -> Picture {
    let mut out = picture.clone();
    let lattice = &stack.lattice;
    let row = picture.width as usize * 4;
    let rows = picture.height as usize;
    if row == 0 || rows == 0 || !picture.is_valid() {
        return out;
    }
    let pixels = &mut out.rgba[..row * rows];
    let run = |band: &mut [u8]| {
        for px in band.as_chunks_mut::<4>().0 {
            if px[3] == 0 {
                continue;
            }
            let v = lattice.lookup([px[0], px[1], px[2]]);
            px[0] = quantize(v[0]);
            px[1] = quantize(v[1]);
            px[2] = quantize(v[2]);
        }
    };
    let bands = threads.clamp(1, rows.div_ceil(16).max(1));
    if bands == 1 {
        run(pixels);
    } else {
        let per = rows.div_ceil(bands);
        std::thread::scope(|scope| {
            for band in pixels.chunks_mut(per * row) {
                let run = &run;
                scope.spawn(move || run(band));
            }
        });
    }
    out
}

/// The compositor's caches: compiled stacks by digest (16), and graded
/// pictures by source picture and digest (8). A source picture is held by
/// its entry, so its address can't be reused while the entry lives.
#[derive(Debug, Default)]
pub(crate) struct GradeCache {
    compiled: Vec<CompiledStack>,
    graded: Vec<(Arc<Picture>, ColorDigest, Arc<Picture>)>,
}

const COMPILED_ENTRIES: usize = 16;
const GRADED_ENTRIES: usize = 8;

impl GradeCache {
    /// `picture` graded by `stack`, from the cache when it was graded
    /// before.
    pub(crate) fn graded(
        &mut self,
        picture: &Arc<Picture>,
        stack: &ColorStack,
        threads: usize,
    ) -> Arc<Picture> {
        let size = BakeSize::for_stack(stack);
        let digest = ColorDigest::of(stack, size);
        if let Some(i) = self
            .graded
            .iter()
            .position(|(src, d, _)| *d == digest && Arc::ptr_eq(src, picture))
        {
            let entry = self.graded.remove(i);
            let out = Arc::clone(&entry.2);
            self.graded.push(entry);
            return out;
        }
        let compiled = match self.compiled.iter().position(|c| c.digest == digest) {
            Some(i) => {
                let c = self.compiled.remove(i);
                self.compiled.push(c.clone());
                c
            }
            None => {
                let c = CompiledStack {
                    digest,
                    lattice: bake_with(stack, size, threads),
                };
                if self.compiled.len() >= COMPILED_ENTRIES {
                    self.compiled.remove(0);
                }
                self.compiled.push(c.clone());
                c
            }
        };
        let out = Arc::new(grade(&compiled, picture, threads));
        if self.graded.len() >= GRADED_ENTRIES {
            self.graded.remove(0);
        }
        self.graded
            .push((Arc::clone(picture), digest, Arc::clone(&out)));
        out
    }

    pub(crate) fn len(&self) -> (usize, usize) {
        (self.compiled.len(), self.graded.len())
    }
}
