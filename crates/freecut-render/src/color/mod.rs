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
//!   never changed. [`grade_in_place`] does the same to a frame's rows (an
//!   adjustment clip, CL-006).
//! - [`ColorDigest`]: what a compiled stack and a graded picture are cached
//!   by.
//! - `curve`, `hsl`, `wheels`: stages 9, 11 and 8 (CL-014, CL-013,
//!   CL-015), with [`CurveTable`], [`hsl_band_at`] and [`probe_hsl_input`]
//!   for the HSL eyedropper.
//! - `matching`, [`stats`]: stage 4, the colour match (CL-020) in Rec.709
//!   `Y'CbCr` of code values, and the pure statistics behind Auto (CL-019)
//!   and Match (`frame_stats`, `auto_color`, `ycc_stats`, `match_stats`,
//!   [`stack_input`]).
//! - `lut`, `skin`: stage 13, the user LUT (CL-017), whose tables come from
//!   a [`LutSource`], and skin protection's mask (CL-030). A stack whose
//!   only active section is the LUT skips the bake and samples the user's
//!   own table (the LUT-only fast path); a LUT its source lacks renders as
//!   identity and is reported.
//! - Colour masks (CL-031): a stack with masks grades every pixel fully,
//!   then blends back toward the pixel by `amount × coverage` before the
//!   single quantization, with coverage from `crate::shape::mask` in the
//!   item's frame ([`MaskSpace`]). Masks are never baked, so a moving mask
//!   never re-bakes the lattice.
//!
//! The stack arrives evaluated (keyed values already worked out by the
//! timeline) on `LayerPlan::color`; `None` there means nothing to do.

mod bake;
mod curve;
mod eval;
mod hsl;
mod lut;
mod matching;
mod skin;
pub mod stats;
mod wheels;

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::sync::Arc;

use freecut_timeline::model::{
    ColorParamId, ColorStack, CurveChannel, LutInterp, LutRef, Section, SkinScope, WheelId,
};

use crate::cpu::Picture;
use crate::shape::mask::{MaskSet, MaskSpace};

pub use bake::{BakeSize, Lattice, bake, bake_eval, bake_with};
pub use curve::{CURVE_TABLE_SIZE, CurveTable};
pub use eval::{Evaluator, LUMA, eval_pixel, eval_pixel_with, linear_to_ws, luma, ws_to_linear};
pub use hsl::{band_weights, hsl_band_at, hue_and_chroma};
pub use lut::{LutSet, LutSource, NoLuts, ResolvedLut, resolve_lut};
pub use matching::{code_from_ycc, ycc_from_code};
pub use skin::skin_mask;
pub use stats::stack_input;
pub use wheels::puck_vector;

/// The version of the colour math. Bump it whenever any stage's arithmetic
/// changes, so caches keyed by [`ColorDigest`] never mix two versions (like
/// the renderer's ABI version). 2: HSL, Curves and Wheels (K2). 3: the
/// user LUT and skin protection (K3). 4: Auto's offsets and the colour
/// match (K5). 5: colour masks (K6).
pub const COLOR_ALGO_VERSION: u32 = 5;

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
        // The Light and Color sliders as the renderer sees them: Auto's
        // offsets and the two sections' switches are already in them.
        let basic = stack.effective_basic();
        for id in ColorParamId::LIGHT.iter().chain(&ColorParamId::COLOR) {
            bytes.extend_from_slice(&basic.get(*id).unwrap_or(0).to_le_bytes());
        }
        for section in [
            Section::Match,
            Section::Light,
            Section::Color,
            Section::Lut,
            Section::Hsl,
            Section::Curves,
            Section::Wheels,
        ] {
            let on = stack.sections_enabled.is_on(section);
            bytes.push(u8::from(on));
            if !on {
                continue;
            }
            if !matches!(section, Section::Light | Section::Color | Section::Match) {
                for id in section.params() {
                    bytes.extend_from_slice(&stack.get(*id).to_le_bytes());
                }
            }
            match section {
                Section::Match => match stack.match_xf.as_ref().filter(|m| m.changes()) {
                    Some(m) => {
                        bytes.push(m.intensity);
                        for v in m.gain().iter().chain(&m.offset()) {
                            bytes.extend_from_slice(&v.to_le_bytes());
                        }
                    }
                    None => bytes.push(0),
                },
                Section::Curves => {
                    for channel in CurveChannel::ALL {
                        let points = stack.curves.get(channel).points();
                        bytes.push(points.len() as u8);
                        for p in points {
                            bytes.extend_from_slice(&p.x.to_le_bytes());
                            bytes.extend_from_slice(&p.y.to_le_bytes());
                        }
                    }
                }
                Section::Wheels => {
                    for w in WheelId::ALL {
                        let (x, y) = stack.puck(w);
                        bytes.extend_from_slice(&x.to_le_bytes());
                        bytes.extend_from_slice(&y.to_le_bytes());
                    }
                }
                Section::Color => {
                    let skin = stack.skin(SkinScope::Color);
                    bytes.push(u8::from(skin.on));
                    bytes.push(skin.amount);
                }
                Section::Lut => match &stack.lut {
                    // The hash names the table's content.
                    Some(slot) => {
                        bytes.push(1);
                        bytes.extend_from_slice(&slot.lut.hash);
                        bytes.push(u8::from(slot.interp == LutInterp::Trilinear));
                        bytes.push(u8::from(slot.skin.on));
                    }
                    None => bytes.push(0),
                },
                _ => {}
            }
        }
        // Colour masks (CL-031), only while they limit the grade; ids
        // don't change the picture.
        bytes.push(u8::from(stack.masks_active()));
        if stack.masks_active() {
            for m in &stack.masks {
                let mask = &m.mask;
                bytes.push(mask.kind() as u8);
                for v in [
                    mask.center.x,
                    mask.center.y,
                    mask.size.x,
                    mask.size.y,
                    mask.rotation.hundredths(),
                    i32::from(mask.feather.get()),
                    i32::from(m.expand),
                ] {
                    bytes.extend_from_slice(&v.to_le_bytes());
                }
                let own = match mask.shape {
                    freecut_timeline::model::MaskShape::Band { width } => width.get(),
                    freecut_timeline::model::MaskShape::Rectangle { roundness } => roundness.get(),
                    _ => 0,
                };
                bytes.extend_from_slice(&own.to_le_bytes());
                bytes.push(u8::from(mask.invert));
                bytes.push(m.mode as u8);
            }
        }
        ColorDigest(xxhash_rust::xxh3::xxh3_128(&bytes))
    }
}

/// How a compiled stack grades a pixel.
#[derive(Debug, Clone, PartialEq)]
enum GradePath {
    /// Every per-pixel stage baked into one lattice.
    Lattice(Arc<Lattice>),
    /// The LUT-only fast path (CL-017): the user's own table, sampled with
    /// its interpolation and domain and blended by Intensity.
    Lut(lut::LutStage),
}

/// A stack ready to grade with: its lattice (or the user's LUT, on the
/// LUT-only path), its colour masks and its digest.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledStack {
    pub digest: ColorDigest,
    path: GradePath,
    /// With colour masks (CL-031): the path grades fully and each pixel is
    /// blended back by `amount × coverage`.
    blend: Option<Arc<MaskBlend>>,
}

/// The final blend of a stack with colour masks (CL-031, stage 19).
#[derive(Debug, Clone, PartialEq)]
struct MaskBlend {
    masks: MaskSet,
    /// Amount (Mix), 0–1.
    amount: f32,
}

/// `stack` split for grading with masks: the stack the path grades (no
/// masks, Amount 100) and the blend; the stack itself and no blend while
/// no mask limits it.
fn split_masks(stack: &ColorStack) -> (Cow<'_, ColorStack>, Option<MaskBlend>) {
    if !stack.masks_active() {
        return (Cow::Borrowed(stack), None);
    }
    let blend = MaskBlend {
        masks: MaskSet::new(&stack.masks),
        amount: f32::from(stack.amount.min(100)) / 100.0,
    };
    let inner = ColorStack {
        masks: Vec::new(),
        amount: 100,
        ..stack.clone()
    };
    (Cow::Owned(inner), Some(blend))
}

/// Default bake threads: up to 8.
fn bake_threads() -> usize {
    std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(8)
}

/// Whether the LUT section is the only one that changes the picture, with
/// no skin protection and Amount 100: the LUT-only fast path (CL-017).
fn lut_only(stack: &ColorStack, eval: &Evaluator) -> bool {
    eval.has_lut()
        && stack.amount >= 100
        && !stack.skin(SkinScope::Lut).is_active()
        && Section::ALL
            .iter()
            .all(|s| *s == Section::Lut || !stack.section_is_active(*s))
}

impl CompiledStack {
    /// Compiles `stack` without LUT tables (a LUT slot grades nothing),
    /// baking on up to 8 threads.
    pub fn new(stack: &ColorStack) -> CompiledStack {
        CompiledStack::with_luts(stack, &NoLuts)
    }

    /// Compiles `stack` with its LUT from `luts`: the LUT-only fast path
    /// when it applies, the lattice otherwise. A missing LUT grades
    /// nothing.
    pub fn with_luts(stack: &ColorStack, luts: &dyn LutSource) -> CompiledStack {
        let (effective, table, _) = effective(stack, luts);
        compile(&effective, table, bake_threads(), true)
    }

    /// [`CompiledStack::with_luts`], always through the lattice (the fast
    /// path's oracle in tests).
    pub fn baked(stack: &ColorStack, luts: &dyn LutSource) -> CompiledStack {
        let (effective, table, _) = effective(stack, luts);
        compile(&effective, table, bake_threads(), false)
    }

    /// The lattice, unless this is the LUT-only path.
    pub fn lattice(&self) -> Option<&Arc<Lattice>> {
        match &self.path {
            GradePath::Lattice(l) => Some(l),
            GradePath::Lut(_) => None,
        }
    }

    /// Whether the user's LUT is sampled directly (CL-017's fast path).
    pub fn is_lut_only(&self) -> bool {
        matches!(self.path, GradePath::Lut(_))
    }

    /// Whether colour masks limit the grade (CL-031).
    pub fn has_masks(&self) -> bool {
        self.blend.is_some()
    }
}

/// `stack` as it grades with `luts`: without its LUT slot when the table is
/// missing (CL-017: the stage renders as identity), with the table looked
/// up, and the missing file.
fn effective<'a>(
    stack: &'a ColorStack,
    luts: &dyn LutSource,
) -> (
    Cow<'a, ColorStack>,
    Option<Arc<freecut_media::lut::Lut>>,
    Option<LutRef>,
) {
    let resolved = resolve_lut(stack, luts);
    match resolved.missing {
        Some(missing) => (Cow::Owned(stack.with_lut(None)), None, Some(missing)),
        None => (Cow::Borrowed(stack), resolved.table, None),
    }
}

fn compile(
    stack: &ColorStack,
    table: Option<Arc<freecut_media::lut::Lut>>,
    threads: usize,
    fast: bool,
) -> CompiledStack {
    let (inner, blend) = split_masks(stack);
    let eval = Evaluator::with_table(&inner, table);
    let size = BakeSize::for_evaluator(&eval);
    let digest = ColorDigest::of(stack, size);
    let fast_stage = if fast && lut_only(&inner, &eval) {
        eval.lut_stage().cloned()
    } else {
        None
    };
    let path = match fast_stage {
        Some(stage) => GradePath::Lut(stage),
        None => GradePath::Lattice(bake_eval(&eval, size, threads)),
    };
    CompiledStack {
        digest,
        path,
        blend: blend.map(Arc::new),
    }
}

/// The pixel as it enters the HSL stage (stage 11), averaged over the
/// opaque pixels of the 5×5 window around `(x, y)` (clipped to the
/// picture): what CL-013's eyedropper samples. `None` when the window
/// holds no opaque pixel or the point lies outside the picture.
pub fn probe_hsl_input(stack: &ColorStack, picture: &Picture, x: u32, y: u32) -> Option<[f32; 3]> {
    if !picture.is_valid() || x >= picture.width || y >= picture.height {
        return None;
    }
    let eval = Evaluator::new(stack);
    let (w, h) = (picture.width as usize, picture.height as usize);
    let (x, y) = (x as usize, y as usize);
    let mut sum = [0.0f32; 3];
    let mut n = 0u32;
    for py in y.saturating_sub(2)..(y + 3).min(h) {
        for px in x.saturating_sub(2)..(x + 3).min(w) {
            let i = (py * w + px) * 4;
            let p = &picture.rgba[i..i + 4];
            if p[3] == 0 {
                continue;
            }
            let v = eval.hsl_input([p[0], p[1], p[2]].map(|c| f32::from(c) / 255.0));
            for c in 0..3 {
                sum[c] += v[c];
            }
            n += 1;
        }
    }
    (n > 0).then(|| sum.map(|v| v / n as f32))
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
/// alpha 0 keep their colour too (CL-001). Colour masks are evaluated in
/// the picture's upright frame; [`grade_displayed`] names the display
/// frame.
pub fn grade(stack: &CompiledStack, picture: &Picture, threads: usize) -> Picture {
    let turned = matches!(
        picture.rotation,
        freecut_timeline::model::Rotation::Cw90 | freecut_timeline::model::Rotation::Cw270
    );
    let upright = if turned {
        (picture.height, picture.width)
    } else {
        (picture.width, picture.height)
    };
    grade_displayed(stack, picture, upright, threads)
}

/// [`grade`] for a picture shown in a display frame `display` big: colour
/// masks live in that frame (CL-031, TR-017's clip space), so they follow
/// the clip's transform and don't depend on the decoded size.
pub fn grade_displayed(
    stack: &CompiledStack,
    picture: &Picture,
    display: (u32, u32),
    threads: usize,
) -> Picture {
    let mut out = picture.clone();
    if picture.is_valid() {
        let n = picture.width as usize * picture.height as usize * 4;
        let space = MaskSpace::picture(
            (picture.width, picture.height),
            picture.rotation,
            picture.mirrored,
            display,
        );
        grade_rows(stack, &mut out.rgba[..n], picture.width, threads, &space);
    }
    out
}

/// Grades straight-alpha RGBA8 rows `width` pixels wide in place, like
/// [`grade`]: an adjustment clip grading the frame drawn so far (CL-006),
/// its colour masks in the frame's own (canvas) space. A trailing partial
/// row is left alone.
pub fn grade_in_place(stack: &CompiledStack, rgba: &mut [u8], width: u32, threads: usize) {
    let rows = rgba.len() / (width as usize * 4).max(1);
    let space = MaskSpace::canvas(width, u32::try_from(rows).unwrap_or(u32::MAX));
    grade_rows(stack, rgba, width, threads, &space);
}

fn grade_rows(
    stack: &CompiledStack,
    rgba: &mut [u8],
    width: u32,
    threads: usize,
    space: &MaskSpace,
) {
    let row = width as usize * 4;
    if row == 0 {
        return;
    }
    let rows = rgba.len() / row;
    if rows == 0 {
        return;
    }
    let pixels = &mut rgba[..row * rows];
    let blend = stack.blend.as_deref();
    let run = |band: &mut [u8], y0: usize| {
        for (i, px) in band.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            if px[3] == 0 {
                continue;
            }
            let v = match &stack.path {
                GradePath::Lattice(lattice) => lattice.lookup([px[0], px[1], px[2]]),
                GradePath::Lut(stage) => {
                    stage.apply([px[0], px[1], px[2]].map(|c| f32::from(c) / 255.0), 0.0)
                }
            };
            let v = match blend {
                None => v,
                Some(b) => {
                    // In range: x < width and y < rows, both from u32 sizes.
                    let (x, y) = (
                        (i % width as usize) as u32,
                        (y0 + i / width as usize) as u32,
                    );
                    let t = b.amount * b.masks.coverage(space.point(x, y), space.pixel());
                    if t >= 1.0 {
                        v
                    } else {
                        let base = [px[0], px[1], px[2]].map(|c| f32::from(c) / 255.0);
                        if t <= 0.0 {
                            base
                        } else {
                            [0, 1, 2].map(|c| base[c] + (v[c] - base[c]) * t)
                        }
                    }
                }
            };
            px[0] = quantize(v[0]);
            px[1] = quantize(v[1]);
            px[2] = quantize(v[2]);
        }
    };
    let bands = threads.clamp(1, rows.div_ceil(16).max(1));
    if bands == 1 {
        run(pixels, 0);
    } else {
        let per = rows.div_ceil(bands);
        std::thread::scope(|scope| {
            for (k, band) in pixels.chunks_mut(per * row).enumerate() {
                let run = &run;
                scope.spawn(move || run(band, k * per));
            }
        });
    }
}

/// A graded picture in the cache: its source picture, digest, display
/// frame (for masks) and the graded result.
type Graded = (Arc<Picture>, ColorDigest, (u32, u32), Arc<Picture>);

/// The compositor's caches: compiled stacks by digest (16), and graded
/// pictures by source picture and digest (8). A source picture is held by
/// its entry, so its address can't be reused while the entry lives. It
/// also collects the LUT files it was asked for and couldn't find.
#[derive(Debug, Default)]
pub(crate) struct GradeCache {
    compiled: Vec<CompiledStack>,
    graded: Vec<Graded>,
    missing: BTreeSet<LutRef>,
}

const COMPILED_ENTRIES: usize = 16;
const GRADED_ENTRIES: usize = 8;

impl GradeCache {
    /// `stack` compiled with `luts`, from the cache when it was compiled
    /// before; `None` when it grades nothing (an identity stack, or one
    /// whose only change is a missing LUT, which is collected).
    pub(crate) fn compiled(
        &mut self,
        stack: &ColorStack,
        luts: &dyn LutSource,
        threads: usize,
    ) -> Option<CompiledStack> {
        let (stack, table, missing) = effective(stack, luts);
        if let Some(missing) = missing {
            self.missing.insert(missing);
        }
        if stack.is_identity() {
            return None;
        }
        Some(self.compile_masked(&stack, table, threads).0)
    }

    /// `stack` compiled, the lattice from the cache: keyed by the stack
    /// without its masks, so a moving mask never re-bakes. Also the digest
    /// of the whole stack.
    fn compile_masked(
        &mut self,
        stack: &ColorStack,
        table: Option<Arc<freecut_media::lut::Lut>>,
        threads: usize,
    ) -> (CompiledStack, ColorDigest) {
        let (inner, blend) = split_masks(stack);
        let eval = Evaluator::with_table(&inner, table.clone());
        let size = BakeSize::for_evaluator(&eval);
        let digest = ColorDigest::of(stack, size);
        let inner_digest = ColorDigest::of(&inner, size);
        let mut compiled = self.compile_cached(&inner, table, inner_digest, threads);
        if let Some(blend) = blend {
            compiled.digest = digest;
            compiled.blend = Some(Arc::new(blend));
        }
        (compiled, digest)
    }

    fn compile_cached(
        &mut self,
        stack: &ColorStack,
        table: Option<Arc<freecut_media::lut::Lut>>,
        digest: ColorDigest,
        threads: usize,
    ) -> CompiledStack {
        match self.compiled.iter().position(|c| c.digest == digest) {
            Some(i) => {
                let c = self.compiled.remove(i);
                self.compiled.push(c.clone());
                c
            }
            None => {
                let c = compile(stack, table, threads, true);
                if self.compiled.len() >= COMPILED_ENTRIES {
                    self.compiled.remove(0);
                }
                self.compiled.push(c.clone());
                c
            }
        }
    }

    /// `picture`, shown in a `display` frame, graded by `stack`, from the
    /// cache when it was graded before.
    pub(crate) fn graded(
        &mut self,
        picture: &Arc<Picture>,
        display: (u32, u32),
        stack: &ColorStack,
        luts: &dyn LutSource,
        threads: usize,
    ) -> Arc<Picture> {
        let (stack, table, missing) = effective(stack, luts);
        if let Some(missing) = missing {
            self.missing.insert(missing);
        }
        if stack.is_identity() {
            return Arc::clone(picture);
        }
        let eval = Evaluator::with_table(&split_masks(&stack).0, table.clone());
        let digest = ColorDigest::of(&stack, BakeSize::for_evaluator(&eval));
        // The display frame matters only to masks.
        let frame = if stack.masks_active() {
            display
        } else {
            (0, 0)
        };
        if let Some(i) = self
            .graded
            .iter()
            .position(|(src, d, f, _)| *d == digest && *f == frame && Arc::ptr_eq(src, picture))
        {
            let entry = self.graded.remove(i);
            let out = Arc::clone(&entry.3);
            self.graded.push(entry);
            return out;
        }
        let (compiled, _) = self.compile_masked(&stack, table, threads);
        let out = Arc::new(grade_displayed(&compiled, picture, display, threads));
        if self.graded.len() >= GRADED_ENTRIES {
            self.graded.remove(0);
        }
        self.graded
            .push((Arc::clone(picture), digest, frame, Arc::clone(&out)));
        out
    }

    pub(crate) fn len(&self) -> (usize, usize) {
        (self.compiled.len(), self.graded.len())
    }

    /// The LUT files asked for since the last call that their source
    /// didn't have, each once, in hash order.
    pub(crate) fn take_missing(&mut self) -> Vec<LutRef> {
        std::mem::take(&mut self.missing).into_iter().collect()
    }
}
