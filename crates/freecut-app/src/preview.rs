//! The preview renderer: turns frame plans into pictures on its own thread.
//!
//! Rendering is `freecut_render`'s CPU compositor, the same renderer export
//! uses (C44), fed by the shared media-backed [`MediaImages`]. Frames are
//! planned the way export plans them (`Compositor::plan` with the preset
//! catalogue), so keyframes and In / Loop / Out animations look the same. Requests are
//! latest-wins: while one frame renders, newer requests replace older ones,
//! so scrubbing never queues up.
//!
//! Each frame also reports where every visible clip landed ([`LayerBox`]),
//! so the player can hit-test and outline clips (TR-007) without laying out
//! text on the UI thread.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use freecut_render::color::LutSource;
use freecut_render::media::{MediaFile, MediaImages};
use freecut_render::{
    Compositor, FontRegistry, ImageFailure, LayerImages, Picture, TextLayer, TextSpec, fit_size,
};
use freecut_timeline::glam::{DMat3, DVec2};
use freecut_timeline::ids::{ClipId, MediaId, TrackId};
use freecut_timeline::model::{Document, LutRef, RichText, TextClipData};
use freecut_timeline::motion::PresetTable;
use freecut_timeline::plan::{FramePlan, LayerPlan, LayerSource, PlanInputs, PlanNode};
use freecut_timeline::time::Tick;

/// Text typed into the inspector but not committed yet (TX-002): the
/// preview shows it in place of the clip's stored content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextDraft {
    pub clip: ClipId,
    pub content: String,
}

/// What to render.
#[derive(Debug, Clone)]
pub struct PreviewRequest {
    pub doc: Arc<Document>,
    pub media_paths: Arc<BTreeMap<MediaId, PathBuf>>,
    pub offline: Arc<BTreeSet<MediaId>>,
    pub time: Tick,
    /// The largest output size wanted; the canvas aspect is kept.
    pub max_size: (u32, u32),
    /// An uncommitted text edit to show instead of the stored content.
    pub text_draft: Option<TextDraft>,
    /// Increases with every request; results carry it back.
    pub generation: u64,
}

/// Where a visible clip landed on the canvas: the rectangle
/// `(0, 0)..size` in its own coordinates, mapped to canvas pixels by
/// `to_canvas`. Pictures use their display size; text uses its laid-out
/// box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerBox {
    pub clip: ClipId,
    pub track: TrackId,
    pub to_canvas: DMat3,
    pub size: (f64, f64),
    /// 0..=1.
    pub opacity: f64,
}

/// A rendered frame, straight RGBA, plus the layers it shows.
#[derive(Debug, Clone)]
pub struct PreviewFrame {
    pub generation: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// The canvas size the layers' coordinates are in.
    pub canvas: (u32, u32),
    /// Bottom to top.
    pub layers: Vec<LayerBox>,
    /// The LUT files this frame named and the LUT source couldn't give
    /// (CL-017's missing badge).
    pub missing_luts: Vec<LutRef>,
}

type Slot = Arc<(Mutex<SlotState>, Condvar)>;

#[derive(Default)]
struct SlotState {
    request: Option<PreviewRequest>,
    /// A new LUT source for the compositor, taken with the next request.
    luts: Option<Arc<dyn LutSource>>,
    stop: bool,
    frame: Option<PreviewFrame>,
}

/// Owns the render thread.
pub struct Preview {
    slot: Slot,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Preview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Preview").finish_non_exhaustive()
    }
}

impl Preview {
    /// Starts the render thread, drawing text with `fonts` and animations
    /// from `presets` (the same catalogue export uses, so they match).
    /// `wake` is called after each frame.
    pub fn new(
        fonts: FontRegistry,
        presets: Arc<PresetTable>,
        wake: impl Fn() + Send + 'static,
    ) -> Preview {
        let slot: Slot = Arc::default();
        let worker_slot = Arc::clone(&slot);
        let thread = std::thread::Builder::new()
            .name("freecut-preview".into())
            .spawn(move || run(&worker_slot, fonts, presets, wake))
            .ok();
        Preview { slot, thread }
    }

    /// Replaces any pending request with this one.
    pub fn request(&self, request: PreviewRequest) {
        let (lock, wake) = &*self.slot;
        if let Ok(mut state) = lock.lock() {
            state.request = Some(request);
            wake.notify_one();
        }
    }

    /// Grades user LUTs with `luts` from the next frame on (CL-017).
    pub fn set_luts(&self, luts: Arc<dyn LutSource>) {
        let (lock, _) = &*self.slot;
        if let Ok(mut state) = lock.lock() {
            state.luts = Some(luts);
        }
    }

    /// The newest finished frame, if any.
    pub fn poll(&self) -> Option<PreviewFrame> {
        let (lock, _) = &*self.slot;
        lock.lock().ok()?.frame.take()
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        let (lock, wake) = &*self.slot;
        if let Ok(mut state) = lock.lock() {
            state.stop = true;
            wake.notify_one();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(slot: &Slot, fonts: FontRegistry, presets: Arc<PresetTable>, wake: impl Fn()) {
    let mut renderer = Renderer::new(fonts, presets);
    loop {
        let request = {
            let (lock, cvar) = &**slot;
            let Ok(mut state) = lock.lock() else { return };
            loop {
                if state.stop {
                    return;
                }
                if let Some(luts) = state.luts.take() {
                    renderer.compositor.set_luts(luts);
                }
                if let Some(r) = state.request.take() {
                    break r;
                }
                state = match cvar.wait(state) {
                    Ok(s) => s,
                    Err(_) => return,
                };
            }
        };
        let frame = renderer.render(&request);
        let (lock, _) = &**slot;
        let Ok(mut state) = lock.lock() else { return };
        if state.stop {
            return;
        }
        // A hidden or busy window retains only the newest finished frame.
        state.frame = Some(frame);
        drop(state);
        wake();
    }
}

/// The compositor and the decoders, kept across frames.
struct Renderer {
    compositor: Compositor,
    images: MediaImages,
    boxes: TextBoxes,
    files_doc: Option<Arc<Document>>,
    files_paths: Arc<BTreeMap<MediaId, PathBuf>>,
    presets: Arc<PresetTable>,
}

impl Renderer {
    fn new(fonts: FontRegistry, presets: Arc<PresetTable>) -> Renderer {
        Renderer {
            compositor: Compositor::new(fonts),
            images: MediaImages::new(),
            boxes: TextBoxes::default(),
            files_doc: None,
            files_paths: Arc::default(),
            presets,
        }
    }

    fn render(&mut self, req: &PreviewRequest) -> PreviewFrame {
        self.sync_files(req);
        let inputs = PlanInputs {
            offline: &req.offline,
        };
        // Planned as export plans (FX-018, TX-022): keyframes, presets and
        // the text boxes this renderer lays out, so preview equals export.
        let plan = self
            .compositor
            .plan(
                &req.doc,
                req.doc.root(),
                req.time,
                &inputs,
                self.presets.as_ref(),
            )
            .ok();
        let canvas = plan
            .as_ref()
            .map_or((16, 9), |p| (p.canvas.width.max(1), p.canvas.height.max(1)));
        let (width, height) = fit_size(canvas, req.max_size);
        let Some(plan) = plan else {
            return PreviewFrame {
                generation: req.generation,
                width,
                height,
                rgba: vec![0; width as usize * height as usize * 4],
                canvas,
                layers: Vec::new(),
                missing_luts: Vec::new(),
            };
        };
        let mut images = WithDraft {
            inner: &mut self.images,
            draft: req.text_draft.as_ref(),
        };
        let frame = self.compositor.render(&plan, width, height, &mut images);
        let layers = layer_boxes(&plan, &mut self.compositor, &mut images, &mut self.boxes);
        PreviewFrame {
            generation: req.generation,
            width: frame.width,
            height: frame.height,
            rgba: frame.rgba,
            canvas,
            layers,
            missing_luts: self.compositor.take_missing_luts(),
        }
    }

    /// Gives the decoders this request's media files. Decoders of files
    /// that didn't change are kept.
    fn sync_files(&mut self, req: &PreviewRequest) {
        if self
            .files_doc
            .as_ref()
            .is_some_and(|doc| Arc::ptr_eq(doc, &req.doc))
            && Arc::ptr_eq(&self.files_paths, &req.media_paths)
        {
            return;
        }
        self.images
            .set_files(MediaFile::from_library(req.doc.library(), &req.media_paths));
        self.files_doc = Some(Arc::clone(&req.doc));
        self.files_paths = Arc::clone(&req.media_paths);
    }
}

/// Pictures from the decoders; text from the plan, with the inspector's
/// uncommitted draft in place of the stored content. Everything else is
/// forwarded, so small layers get the decoders' smaller pictures.
struct WithDraft<'a, I: ?Sized> {
    inner: &'a mut I,
    draft: Option<&'a TextDraft>,
}

impl<I: LayerImages + ?Sized> LayerImages for WithDraft<'_, I> {
    fn begin_frame(&mut self, plan: &FramePlan) {
        self.inner.begin_frame(plan);
    }

    fn video(&mut self, asset: MediaId, source_time: Tick) -> Option<Arc<Picture>> {
        self.inner.video(asset, source_time)
    }

    fn video_scaled(
        &mut self,
        asset: MediaId,
        source_time: Tick,
        fraction: (f64, f64),
    ) -> Option<Arc<Picture>> {
        self.inner.video_scaled(asset, source_time, fraction)
    }

    fn still(&mut self, asset: MediaId) -> Option<Arc<Picture>> {
        self.inner.still(asset)
    }

    fn take_failures(&mut self) -> Vec<ImageFailure> {
        self.inner.take_failures()
    }

    fn text(&mut self, layer: &LayerPlan) -> Option<TextLayer> {
        match (self.draft, &layer.source) {
            (Some(draft), LayerSource::Text { data, scale_px }) if draft.clip == layer.clip => {
                with_content(layer, data, *scale_px, &draft.content)
            }
            _ => TextLayer::from_plan(layer),
        }
    }
}

/// `layer`'s text layer showing `content` instead of its own.
fn with_content(
    layer: &LayerPlan,
    data: &TextClipData,
    scale_px: f64,
    content: &str,
) -> Option<TextLayer> {
    let data = TextClipData {
        content: RichText::new(content),
        ..data.clone()
    };
    let source = LayerSource::Text {
        data: Arc::new(data),
        scale_px,
    };
    TextLayer::from_plan(&LayerPlan {
        source,
        ..layer.clone()
    })
}

/// Where each layer of `plan` landed. Text boxes are measured with the
/// compositor's own text renderer, so they match what was drawn.
fn layer_boxes(
    plan: &FramePlan,
    compositor: &mut Compositor,
    images: &mut dyn LayerImages,
    boxes: &mut TextBoxes,
) -> Vec<LayerBox> {
    let generation = compositor.text_renderer().registry().generation();
    let layers: Vec<LayerBox> = plan
        .nodes
        .iter()
        .filter_map(|node| {
            let PlanNode::Layer(layer) = node else {
                return None;
            };
            let (to_canvas, size) = match &layer.source {
                LayerSource::Text { .. } => {
                    let text = images.text(layer)?;
                    let size = boxes.measure(layer.clip, generation, &text.spec, |spec| {
                        text_box(compositor, spec)
                    });
                    // A text layer's origin is the box's horizontal centre
                    // and the edge its vertical anchor names (TX-005).
                    let (ax, ay) = text.anchor.point((size.0 as f32, size.1 as f32));
                    let to_canvas = text.layer_to_canvas
                        * DMat3::from_translation(-DVec2::new(f64::from(ax), f64::from(ay)));
                    (to_canvas, size)
                }
                _ => (
                    layer.layer_to_canvas,
                    (
                        f64::from(layer.display_size.0),
                        f64::from(layer.display_size.1),
                    ),
                ),
            };
            Some(LayerBox {
                clip: layer.clip,
                track: layer.track,
                to_canvas,
                size,
                opacity: layer.opacity,
            })
        })
        .collect();
    boxes.keep(&layers);
    layers
}

/// The text boxes measured for the last frames, by clip. Laying text out
/// is not cached by the renderer (only rasters are), so a title that stays
/// the same is measured once, not on every frame of playback or scrubbing.
#[derive(Debug, Default)]
struct TextBoxes {
    /// The font set's generation and the spec each box was measured for.
    boxes: HashMap<ClipId, (u64, TextSpec, (f64, f64))>,
}

impl TextBoxes {
    /// `clip`'s box for `spec` with the font set of `generation`: the one
    /// measured before when nothing changed, else `measure`'s.
    fn measure(
        &mut self,
        clip: ClipId,
        generation: u64,
        spec: &TextSpec,
        measure: impl FnOnce(&TextSpec) -> (f64, f64),
    ) -> (f64, f64) {
        if let Some((g, s, size)) = self.boxes.get(&clip)
            && *g == generation
            && s == spec
        {
            return *size;
        }
        let size = measure(spec);
        self.boxes.insert(clip, (generation, spec.clone(), size));
        size
    }

    /// Forgets the clips not in this frame.
    fn keep(&mut self, layers: &[LayerBox]) {
        self.boxes
            .retain(|clip, _| layers.iter().any(|l| l.clip == *clip));
    }
}

/// The laid-out size of `spec`, in canvas pixels at 100 % scale.
fn text_box(compositor: &mut Compositor, spec: &TextSpec) -> (f64, f64) {
    let layout = compositor.text_renderer().layout(spec);
    (f64::from(layout.size.0), f64::from(layout.size.1))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use std::num::NonZeroU64;

    use super::*;

    /// Records which provider methods were called.
    #[derive(Default)]
    struct Spy {
        scaled: Vec<(f64, f64)>,
        taken: usize,
    }

    impl LayerImages for Spy {
        fn video(&mut self, _: MediaId, _: Tick) -> Option<Arc<Picture>> {
            None
        }

        fn video_scaled(&mut self, _: MediaId, _: Tick, f: (f64, f64)) -> Option<Arc<Picture>> {
            self.scaled.push(f);
            None
        }

        fn still(&mut self, _: MediaId) -> Option<Arc<Picture>> {
            None
        }

        fn take_failures(&mut self) -> Vec<ImageFailure> {
            self.taken += 1;
            Vec::new()
        }
    }

    /// S1: a text box that didn't change is measured once across frames;
    /// a new spec or font set measures again, and clips that left the
    /// frame are forgotten.
    #[test]
    fn unchanged_text_boxes_are_measured_once() {
        let clip = ClipId::from_raw(NonZeroU64::MIN);
        let spec = TextSpec::new("Title");
        let mut boxes = TextBoxes::default();
        let measured = std::cell::Cell::new(0);
        let measure = |spec: &TextSpec, generation: u64, boxes: &mut TextBoxes| {
            boxes.measure(clip, generation, spec, |_| {
                measured.set(measured.get() + 1);
                (100.0, 40.0)
            })
        };
        for _ in 0..30 {
            assert_eq!(measure(&spec, 1, &mut boxes), (100.0, 40.0));
        }
        assert_eq!(measured.get(), 1);
        let mut longer = spec.clone();
        longer.text.push('!');
        measure(&longer, 1, &mut boxes);
        measure(&longer, 2, &mut boxes);
        assert_eq!(measured.get(), 3);
        boxes.keep(&[]);
        assert!(boxes.boxes.is_empty());
    }

    #[test]
    fn the_draft_wrapper_forwards_small_decodes_and_failures() {
        let mut spy = Spy::default();
        let mut images = WithDraft {
            inner: &mut spy,
            draft: None,
        };
        let asset = MediaId::from_raw(NonZeroU64::MIN);
        images.video_scaled(asset, Tick::ZERO, (0.25, 0.25));
        images.take_failures();
        assert_eq!(spy.scaled, [(0.25, 0.25)]);
        assert_eq!(spy.taken, 1);
    }
}
