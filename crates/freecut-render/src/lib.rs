//! Rendering for FreeCut: everything drawn into a video frame.
//!
//! - [`cpu`]: the CPU compositor. It turns a timeline
//!   [`FramePlan`](freecut_timeline::plan::FramePlan) into RGBA pixels and
//!   is both the preview and the export renderer today, and the reference
//!   the planned wgpu compositor must match.
//! - [`text`]: fonts (bundled Noto Sans plus system fonts), layout with
//!   parley and glyph rasterization with swash.
//! - [`media`] (feature `media`, on by default): a [`cpu::LayerImages`]
//!   backed by `freecut-media` decoders, shared by preview and export.
//! - [`shape`]: vector shapes (boxes, bubbles, decorations) filled into
//!   coverage masks.
//! - [`export`]: export frames on the export clock's grid, fitted into the
//!   output size and composited over black.
//! - [`motion`]: frames planned with the animation catalogue, and the
//!   timeline's per-unit text animations turned into render requests.
//! - [`color`]: the colour stack (CL-001) on the CPU path: the reference
//!   evaluator, the baked lattice and grading.
#![forbid(unsafe_code)]

pub mod color;
pub mod cpu;
pub mod export;
#[cfg(feature = "media")]
pub mod media;
pub mod motion;
pub mod shape;
pub mod text;

pub use cpu::{
    Compositor, Frame, ImageFailure, LayerImages, NoImages, Picture, TextAnchor, TextLayer,
    fit_size,
};
pub use export::{ExportRenderError, ExportRenderer, UnreadableFrame, fit_rect};
pub use text::{FontRegistry, TextAlign, TextRaster, TextRenderer, TextSpec};
mod parallel;
