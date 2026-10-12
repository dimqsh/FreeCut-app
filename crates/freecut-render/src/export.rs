//! Export frames: the CPU compositor driven by the export clock (EX-002).
//!
//! [`ExportRenderer`] renders output frame `k` of an export: it plans the
//! sequence at [`ExportClock::frame_time`]`(k)`, renders the plan with the
//! same [`Compositor`] the preview uses, fits the canvas into the output
//! size (aspect kept, scaled up or down, centred, black bars around it) and
//! composites the result over black, so every pixel is opaque RGBA8 ready
//! for the encoder.
//!
//! **Same pixels as the preview.** Inside the picture rectangle the bytes
//! are exactly what the preview path (`Compositor::render` of the same plan
//! at the rectangle's size) produces, followed by "over black", which
//! changes nothing for an opaque canvas.
//!
//! **Animations.** Frames are planned with the preset catalogue given by
//! [`ExportRenderer::with_presets`] (none by default: keyframes play,
//! animations don't) and text boxes measured by the compositor's layout
//! ([`Compositor::plan`]), the same planning the preview uses.
//!
//! **Determinism.** The output depends only on the document, the clock, the
//! output size, the presets and the pictures: the compositor is deterministic whatever
//! its thread count, and [`ExportRenderer::new`] waits for a running system
//! font scan so the font set can't change halfway through an export.
//!
//! **Decoding.** Frames are requested in increasing time order, so the
//! media-backed [`LayerImages`] (`crate::media::MediaImages`, one decoder per
//! asset) decodes each asset forward instead of seeking.
//!
//! **Unreadable sources** (EX-022 `decode-error`). A picture the provider
//! couldn't open or decode ([`LayerImages::take_failures`]) stops the
//! export: [`ExportRenderer::render_frame`] returns
//! [`ExportRenderError::Decode`] naming the clip and timeline time. After
//! [`ExportRenderer::with_slates_for_unreadable`] ("Retry using slates")
//! those frames draw the offline slate instead and are listed in
//! [`ExportRenderer::unreadable`] for the result view. Offline media (no
//! file at all) always draws the slate (EX-005) and is not a failure.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use freecut_timeline::export::{ExportClock, ExportTimeError};
use freecut_timeline::ids::{ClipId, MediaId, SeqId};
use freecut_timeline::model::Document;
use freecut_timeline::motion::{NoPresets, PresetCatalog};
use freecut_timeline::plan::{FramePlan, LayerSource, PlanError, PlanInputs, PlanNode};
use freecut_timeline::time::Tick;

use crate::cpu::{Compositor, Frame, ImageFailure, LayerImages};

/// Largest output side accepted.
pub const MAX_OUTPUT_SIDE: u32 = 16_384;

/// How long [`ExportRenderer::new`] waits for a running system font scan.
pub const FONT_SCAN_TIMEOUT: Duration = Duration::from_secs(30);

/// Why an export frame can't be rendered.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExportRenderError {
    #[error("sequence {0} doesn't exist")]
    NoSequence(SeqId),
    #[error("output size {width}x{height} is not usable")]
    InvalidSize { width: u32, height: u32 },
    #[error("frame {frame} is past the end of the export ({count} frames)")]
    FrameOutOfRange { frame: u64, count: u64 },
    #[error(transparent)]
    Time(#[from] ExportTimeError),
    #[error(transparent)]
    Plan(#[from] PlanError),
    /// A source picture couldn't be opened or decoded (EX-022
    /// `decode-error`). `clip` is the clip showing it in that frame.
    #[error("can't read the picture of clip {} at {at}: {reason}", .clip.map_or_else(|| "?".to_owned(), |c| c.to_string()))]
    Decode {
        clip: Option<ClipId>,
        /// Timeline time of the frame.
        at: Tick,
        reason: String,
    },
}

/// An export frame drawn with the slate because a source picture couldn't
/// be read (after [`ExportRenderer::with_slates_for_unreadable`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadableFrame {
    /// Output frame index.
    pub frame: u64,
    pub clip: Option<ClipId>,
    /// Timeline time of the frame.
    pub at: Tick,
    pub reason: String,
}

/// A pixel rectangle `x, y, width, height`.
pub type Rect = (u32, u32, u32, u32);

/// Where a `canvas` lands in an `output` frame: as large as fits with its
/// aspect kept (scaled up or down), centred. Sizes are rounded to whole
/// pixels; with the same aspect it is the whole output. Exact integer
/// arithmetic.
pub fn fit_rect(canvas: (u32, u32), output: (u32, u32)) -> Rect {
    let (cw, ch) = (u64::from(canvas.0.max(1)), u64::from(canvas.1.max(1)));
    let (ow, oh) = (u64::from(output.0), u64::from(output.1));
    // Each quotient is at most `ow` or `oh`, so it fits in u32.
    let narrow = |num: u64, den: u64, limit: u64| ((2 * num + den) / (2 * den)).clamp(1, limit);
    let (w, h) = if ow * ch <= oh * cw {
        (ow, narrow(ow * ch, cw, oh.max(1)))
    } else {
        (narrow(oh * cw, ch, ow.max(1)), oh)
    };
    let to_u32 = |v: u64| u32::try_from(v).unwrap_or(u32::MAX);
    (
        to_u32(ow.saturating_sub(w) / 2),
        to_u32(oh.saturating_sub(h) / 2),
        to_u32(w),
        to_u32(h),
    )
}

/// Renders the frames of one export, in any order (increasing order is the
/// fast path for decoding).
pub struct ExportRenderer<I> {
    doc: Arc<Document>,
    seq: SeqId,
    clock: ExportClock,
    rect: Rect,
    offline: BTreeSet<MediaId>,
    /// The animation catalogue (FX-030, TX-056).
    presets: Arc<dyn PresetCatalog + Send + Sync>,
    compositor: Compositor,
    images: I,
    /// The canvas rendered at the rectangle's size, when that isn't the
    /// whole output.
    picture: Frame,
    /// The output frame. Its bars are painted once.
    frame: Frame,
    /// Draw slates for unreadable pictures instead of failing.
    slates_for_unreadable: bool,
    unreadable: Vec<UnreadableFrame>,
}

impl<I> std::fmt::Debug for ExportRenderer<I> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExportRenderer")
            .field("seq", &self.seq)
            .field("clock", &self.clock)
            .field("output", &(self.frame.width, self.frame.height))
            .field("rect", &self.rect)
            .field("offline", &self.offline.len())
            .finish_non_exhaustive()
    }
}

impl<I: LayerImages> ExportRenderer<I> {
    /// Renders sequence `seq` of `doc` on `clock`'s grid at `output` size,
    /// with pictures from `images`.
    ///
    /// Blocks until a running system font scan of the compositor's registry
    /// has finished (at most [`FONT_SCAN_TIMEOUT`]): call it on the export
    /// thread.
    pub fn new(
        doc: Arc<Document>,
        seq: SeqId,
        clock: ExportClock,
        output: (u32, u32),
        mut compositor: Compositor,
        images: I,
    ) -> Result<ExportRenderer<I>, ExportRenderError> {
        let (width, height) = output;
        if width == 0 || height == 0 || width > MAX_OUTPUT_SIDE || height > MAX_OUTPUT_SIDE {
            return Err(ExportRenderError::InvalidSize { width, height });
        }
        if doc.sequence(seq).is_none() {
            return Err(ExportRenderError::NoSequence(seq));
        }
        compositor
            .text_renderer()
            .registry()
            .wait_for_scan(FONT_SCAN_TIMEOUT);
        let settings = doc.settings();
        let rect = fit_rect((settings.width, settings.height), output);
        let mut frame = Frame::new(width, height);
        for px in frame.rgba.as_chunks_mut::<4>().0 {
            *px = [0, 0, 0, 255];
        }
        Ok(ExportRenderer {
            doc,
            seq,
            clock,
            rect,
            offline: BTreeSet::new(),
            presets: Arc::new(NoPresets),
            compositor,
            images,
            picture: Frame::new(rect.2, rect.3),
            frame,
            slates_for_unreadable: false,
            unreadable: Vec::new(),
        })
    }

    /// "Retry using slates for unreadable frames" (EX-022): pictures that
    /// can't be read draw the offline slate instead of stopping the export,
    /// and are listed in [`ExportRenderer::unreadable`].
    pub fn with_slates_for_unreadable(mut self) -> ExportRenderer<I> {
        self.slates_for_unreadable = true;
        self
    }

    /// Frames drawn with slates for unreadable pictures so far, in render
    /// order (always empty without
    /// [`ExportRenderer::with_slates_for_unreadable`]).
    pub fn unreadable(&self) -> &[UnreadableFrame] {
        &self.unreadable
    }

    /// Media whose file is unavailable: drawn as the offline slate (EX-005).
    pub fn with_offline(mut self, offline: BTreeSet<MediaId>) -> ExportRenderer<I> {
        self.offline = offline;
        self
    }

    /// Plays the animations of `presets` (FX-018, TX-022); a preset
    /// missing from it plays as no animation.
    pub fn with_presets(
        mut self,
        presets: Arc<dyn PresetCatalog + Send + Sync>,
    ) -> ExportRenderer<I> {
        self.presets = presets;
        self
    }

    pub fn clock(&self) -> &ExportClock {
        &self.clock
    }

    pub fn frame_count(&self) -> u64 {
        self.clock.frame_count()
    }

    pub fn output_size(&self) -> (u32, u32) {
        (self.frame.width, self.frame.height)
    }

    /// Where the canvas lands in each output frame ([`fit_rect`]).
    pub fn picture_rect(&self) -> Rect {
        self.rect
    }

    pub fn images_mut(&mut self) -> &mut I {
        &mut self.images
    }

    /// The plan of output frame `k`.
    pub fn plan(&mut self, k: u64) -> Result<FramePlan, ExportRenderError> {
        let count = self.clock.frame_count();
        if k >= count {
            return Err(ExportRenderError::FrameOutOfRange { frame: k, count });
        }
        let t = self.clock.frame_time(k)?;
        let inputs = PlanInputs {
            offline: &self.offline,
        };
        Ok(self
            .compositor
            .plan(&self.doc, self.seq, t, &inputs, self.presets.as_ref())?)
    }

    /// Output frame `k`: opaque RGBA8 at the output size.
    ///
    /// Fails with [`ExportRenderError::Decode`] when a source picture of
    /// the frame can't be read, unless slates were asked for.
    pub fn render_frame(&mut self, k: u64) -> Result<&Frame, ExportRenderError> {
        let plan = self.plan(k)?;
        // Failures left over from earlier use of the provider aren't this
        // frame's.
        drop(self.images.take_failures());
        let (x, y, w, h) = self.rect;
        if (w, h) == (self.frame.width, self.frame.height) {
            self.compositor
                .render_into(&plan, &mut self.frame, &mut self.images);
            self.check_failures(k, &plan)?;
            over_black(&mut self.frame.rgba);
            return Ok(&self.frame);
        }
        self.compositor
            .render_into(&plan, &mut self.picture, &mut self.images);
        self.check_failures(k, &plan)?;
        over_black(&mut self.picture.rgba);
        let src_row = w as usize * 4;
        let dst_row = self.frame.width as usize * 4;
        let left = x as usize * 4;
        for (row, src) in self.picture.rgba.chunks_exact(src_row).enumerate() {
            let start = (y as usize + row) * dst_row + left;
            if let Some(dst) = self.frame.rgba.get_mut(start..start + src_row) {
                dst.copy_from_slice(src);
            }
        }
        Ok(&self.frame)
    }
}

impl<I: LayerImages> ExportRenderer<I> {
    /// Turns the provider's failures while rendering frame `k` into an
    /// error, or records them when slates were asked for.
    fn check_failures(&mut self, k: u64, plan: &FramePlan) -> Result<(), ExportRenderError> {
        let failures = self.images.take_failures();
        let Some(first) = failures.first() else {
            return Ok(());
        };
        if !self.slates_for_unreadable {
            return Err(ExportRenderError::Decode {
                clip: clip_showing(plan, first),
                at: plan.time,
                reason: first.reason.clone(),
            });
        }
        for failure in &failures {
            let clip = clip_showing(plan, failure);
            if !self
                .unreadable
                .iter()
                .any(|u| u.frame == k && u.clip == clip)
            {
                self.unreadable.push(UnreadableFrame {
                    frame: k,
                    clip,
                    at: plan.time,
                    reason: failure.reason.clone(),
                });
            }
        }
        Ok(())
    }
}

/// The topmost clip of `plan` showing the failed asset (at the failed
/// media time, when there are several).
fn clip_showing(plan: &FramePlan, failure: &ImageFailure) -> Option<ClipId> {
    let mut fallback = None;
    for node in plan.nodes.iter().rev() {
        let PlanNode::Layer(layer) = node;
        match &layer.source {
            LayerSource::Video { asset, source_time } if *asset == failure.asset => {
                if failure.source_time == Some(*source_time) {
                    return Some(layer.clip);
                }
                fallback = fallback.or(Some(layer.clip));
            }
            LayerSource::Still { asset } if *asset == failure.asset => {
                fallback = fallback.or(Some(layer.clip));
            }
            _ => {}
        }
    }
    fallback
}

#[cfg(feature = "media")]
impl ExportRenderer<crate::media::MediaImages> {
    /// An export renderer decoding the library's files: `paths` gives the
    /// resolved file of each media item (items without one draw the
    /// offline slate). Frames are decoded exactly (`SeekMode::Exact`).
    pub fn with_media(
        doc: Arc<Document>,
        seq: SeqId,
        clock: ExportClock,
        output: (u32, u32),
        compositor: Compositor,
        paths: &std::collections::BTreeMap<MediaId, std::path::PathBuf>,
    ) -> Result<ExportRenderer<crate::media::MediaImages>, ExportRenderError> {
        use crate::media::{MediaFile, MediaImages};
        let mut images = MediaImages::new();
        images.set_files(MediaFile::from_library(doc.library(), paths));
        images.set_seek_mode(freecut_media::SeekMode::Exact);
        ExportRenderer::new(doc, seq, clock, output, compositor, images)
    }
}

/// Composites straight-alpha RGBA over opaque black, in place.
fn over_black(rgba: &mut [u8]) {
    for px in rgba.as_chunks_mut::<4>().0 {
        let a = u32::from(px[3]);
        if a != 255 {
            for c in &mut px[..3] {
                // (c·a + 127) / 255 <= 255.
                *c = ((u32::from(*c) * a + 127) / 255) as u8;
            }
            px[3] = 255;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_aspect_fills_the_output() {
        assert_eq!(fit_rect((1920, 1080), (1280, 720)), (0, 0, 1280, 720));
        assert_eq!(fit_rect((1920, 1080), (3840, 2160)), (0, 0, 3840, 2160));
    }

    #[test]
    fn other_aspects_get_centred_bars() {
        // Wide canvas in a square: bars top and bottom.
        assert_eq!(fit_rect((1920, 1080), (400, 400)), (0, 87, 400, 225));
        // Tall canvas in a wide output: bars left and right.
        assert_eq!(fit_rect((1080, 1920), (1920, 1080)), (656, 0, 608, 1080));
        // Never thinner than a pixel.
        assert_eq!(fit_rect((10_000, 2), (4, 4)), (0, 1, 4, 1));
    }

    #[test]
    fn over_black_premultiplies_and_makes_opaque() {
        let mut px = [200, 100, 0, 128, 9, 9, 9, 255, 255, 255, 255, 0];
        over_black(&mut px);
        assert_eq!(px, [100, 50, 0, 255, 9, 9, 9, 255, 0, 0, 0, 255]);
    }
}
