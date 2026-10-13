//! Preset tiles' pictures for the Motion lens (ui-concept §13.7, UI-007,
//! TX-056): FreeCut's own sample, "Abc", playing each preset, rendered by
//! `freecut_render`'s compositor on a thread of its own, never on the UI
//! thread.
//!
//! Every tile has a still poster (frame 0): the pose 60 % of the way
//! through the motion (TX-022's key pose, also what Reduce motion shows).
//! While one tile is hovered its short loop is rendered at
//! [`TILE_FPS`], one tile at a time: asking for another tile's frames
//! drops the frames still queued for the previous one. Stills are queued
//! separately and are never dropped.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use crossbeam_channel::{Receiver, Sender, unbounded};
use freecut_render::media::MediaImages;
use freecut_render::{Compositor, FontRegistry, fit_size};
use freecut_timeline::edit::{EditRequest, TextStyleEdit};
use freecut_timeline::history::Coalesce;
use freecut_timeline::model::{
    AnimationSpec, Document, PresetId, ProjectSettings, RefPx, RichText, Slot,
};
use freecut_timeline::motion::{
    PresetCatalog, PresetKind, PresetManifest, PresetTable, with_paced_duration,
};
use freecut_timeline::plan::PlanInputs;
use freecut_timeline::time::{TICKS_PER_SECOND, Tick};

use crate::session::Project;

/// The tile's picture size in pixels: 88 × 66 pt at twice the density.
pub const POSTER_SIZE: (u32, u32) = (176, 132);
/// The frame rate of a hovered tile's loop (ui-concept §12.8 asks for at
/// most 24; tiles use a low rate to stay cheap).
pub const TILE_FPS: i64 = 12;
/// The most frames one tile's loop has (4 s at [`TILE_FPS`]).
pub const MAX_FRAMES: u16 = 48;
/// The sample clip's length.
const SAMPLE_SECS: i64 = 3;

/// One picture of one tile: frame 0 is the still poster, frames 1.. the
/// hovered loop.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PosterKey {
    pub preset: PresetId,
    pub slot: Slot,
    pub frame: u16,
}

/// A rendered tile picture, straight RGBA.
#[derive(Debug, Clone)]
pub struct PosterImage {
    pub key: PosterKey,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The sample clip's local times a tile shows: the still, and the start
/// and count of the hovered loop's frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileTimes {
    pub still: Tick,
    pub start: Tick,
    pub frames: u16,
}

/// When the sample plays `manifest` in `slot` with `duration` (its length,
/// or its cycle for loops). In runs from the clip's start and Out ends at
/// its end, each with half a second of rest around it; a loop plays one
/// cycle; drifts and combined presets the whole sample.
pub fn tile_times(manifest: &PresetManifest, slot: Slot, duration: Tick) -> TileTimes {
    let sample = Tick::from_raw(SAMPLE_SECS * TICKS_PER_SECOND);
    let half = Tick::from_raw(TICKS_PER_SECOND / 2);
    let d = duration.clamp(Tick::from_raw(1), sample);
    let at = |share_milli: i64, of: Tick| Tick::from_raw(of.raw() / 1000 * share_milli);
    let (start, len, still) = match slot {
        Slot::Entrance => (Tick::ZERO, Tick::from_raw(d.raw() + half.raw()), at(600, d)),
        Slot::Exit => {
            let begin = sample.raw() - d.raw();
            let from = (begin - half.raw()).max(0);
            (
                Tick::from_raw(from),
                Tick::from_raw(sample.raw() - from),
                Tick::from_raw(begin + at(600, d).raw()),
            )
        }
        Slot::Continuous => {
            let len = if manifest.kind == PresetKind::Loop {
                d
            } else {
                sample
            };
            (Tick::ZERO, len, at(600, len))
        }
    };
    let step = TICKS_PER_SECOND / TILE_FPS;
    let frames = (len.raw() + step - 1) / step;
    TileTimes {
        still,
        start,
        frames: u16::try_from(frames.clamp(1, i64::from(MAX_FRAMES))).unwrap_or(MAX_FRAMES),
    }
}

/// The times of `manifest`'s tile in `slot`: the sample plays the preset
/// with its default settings (a paced preset as long as its Pace makes
/// [`SAMPLE_TEXT`]), so the loop the UI counts and the frames the worker
/// renders always agree, also on the chosen tile.
pub fn sample_times(manifest: &PresetManifest, slot: Slot) -> TileTimes {
    let spec = manifest.default_spec();
    let len = match slot {
        Slot::Continuous => spec.duration,
        Slot::Entrance | Slot::Exit => {
            let rate = sample_settings().map(|s| s.frame_rate);
            rate.ok()
                .and_then(|r| {
                    let paced = with_paced_duration(&spec, SAMPLE_TEXT, r).ok()?;
                    r.quantize(paced.duration).ok()
                })
                .unwrap_or(spec.duration)
        }
    };
    tile_times(manifest, slot, len)
}

/// The text the sample shows.
pub const SAMPLE_TEXT: &str = "Abc";

/// The sample's settings: a 4:3 canvas.
fn sample_settings() -> Result<ProjectSettings, freecut_timeline::time::TimeError> {
    Ok(ProjectSettings {
        width: 640,
        height: 480,
        ..ProjectSettings::new_default()?
    })
}

impl TileTimes {
    /// The sample time of `frame` (0 is the still).
    pub fn time_of(&self, frame: u16) -> Tick {
        if frame == 0 {
            return self.still;
        }
        let step = TICKS_PER_SECOND / TILE_FPS;
        let k = i64::from((frame - 1) % self.frames.max(1));
        Tick::from_raw(self.start.raw() + k * step)
    }
}

#[derive(Default)]
struct Queue {
    stills: VecDeque<PosterKey>,
    /// The hovered tile's frames, in order.
    motion: VecDeque<PosterKey>,
    stop: bool,
}

type Shared = Arc<(Mutex<Queue>, Condvar)>;

/// Owns the poster thread.
pub struct PosterWorker {
    queue: Shared,
    results: Receiver<PosterImage>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for PosterWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PosterWorker").finish_non_exhaustive()
    }
}

impl PosterWorker {
    /// Starts the thread, drawing text with `fonts` and presets from
    /// `presets`. `wake` is called after each picture.
    pub fn new(
        fonts: FontRegistry,
        presets: Arc<PresetTable>,
        wake: impl Fn() + Send + 'static,
    ) -> PosterWorker {
        let queue: Shared = Arc::default();
        let (tx, rx) = unbounded();
        let shared = Arc::clone(&queue);
        let thread = std::thread::Builder::new()
            .name("freecut-posters".into())
            .spawn(move || run(&shared, &tx, fonts, &presets, wake))
            .ok();
        PosterWorker {
            queue,
            results: rx,
            thread,
        }
    }

    /// Queues a tile's still poster (once).
    pub fn want_still(&self, key: PosterKey) {
        let (lock, wake) = &*self.queue;
        if let Ok(mut q) = lock.lock()
            && !q.stills.contains(&key)
        {
            q.stills.push_back(key);
            wake.notify_one();
        }
    }

    /// Replaces the queued loop frames with `keys` (one tile at a time).
    pub fn want_motion(&self, keys: Vec<PosterKey>) {
        let (lock, wake) = &*self.queue;
        if let Ok(mut q) = lock.lock() {
            q.motion = keys.into();
            wake.notify_one();
        }
    }

    /// The pictures finished since the last call.
    pub fn poll(&self) -> Vec<PosterImage> {
        self.results.try_iter().collect()
    }
}

impl Drop for PosterWorker {
    fn drop(&mut self) {
        let (lock, wake) = &*self.queue;
        if let Ok(mut q) = lock.lock() {
            q.stop = true;
            wake.notify_one();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(
    queue: &Shared,
    results: &Sender<PosterImage>,
    fonts: FontRegistry,
    presets: &PresetTable,
    wake: impl Fn(),
) {
    let mut renderer = Renderer {
        compositor: Compositor::new(fonts),
        images: MediaImages::new(),
        samples: HashMap::new(),
    };
    loop {
        let key = {
            let (lock, cvar) = &**queue;
            let Ok(mut q) = lock.lock() else { return };
            loop {
                if q.stop {
                    return;
                }
                // The hovered tile first: it is what the user looks at.
                if let Some(k) = q.motion.pop_front().or_else(|| q.stills.pop_front()) {
                    break k;
                }
                q = match cvar.wait(q) {
                    Ok(q) => q,
                    Err(_) => return,
                };
            }
        };
        if let Some(image) = renderer.render(&key, presets) {
            if results.send(image).is_err() {
                return;
            }
            wake();
        }
    }
}

/// A preset's sample document and the times its tile shows.
type Sample = (Arc<Document>, TileTimes);

struct Renderer {
    compositor: Compositor,
    images: MediaImages,
    /// The sample document of each (preset, slot), with its tile times.
    samples: HashMap<(PresetId, Slot), Option<Sample>>,
}

impl Renderer {
    fn render(&mut self, key: &PosterKey, presets: &PresetTable) -> Option<PosterImage> {
        let sample = self
            .samples
            .entry((key.preset.clone(), key.slot))
            .or_insert_with(|| sample(&key.preset, key.slot, presets))
            .clone();
        let (doc, times) = sample?;
        let inputs = PlanInputs {
            offline: &Default::default(),
        };
        let t = times.time_of(key.frame);
        let plan = self
            .compositor
            .plan(&doc, doc.root(), t, &inputs, presets)
            .ok()?;
        let canvas = (plan.canvas.width.max(1), plan.canvas.height.max(1));
        let (w, h) = fit_size(canvas, POSTER_SIZE);
        let frame = self.compositor.render(&plan, w, h, &mut self.images);
        Some(PosterImage {
            key: key.clone(),
            width: frame.width,
            height: frame.height,
            rgba: frame.rgba,
        })
    }
}

/// The sample document for `preset` in `slot`: "Abc" on a 4:3 canvas,
/// three seconds long, with the preset's default settings.
pub fn sample(
    preset: &PresetId,
    slot: Slot,
    presets: &PresetTable,
) -> Option<(Arc<Document>, TileTimes)> {
    let manifest = presets.resolve(slot, preset)?;
    let mut project = Project::new(sample_settings().ok()?).ok()?;
    let clip = crate::ui::text::add_text_at_playhead(&mut project).ok()?;
    let seq = project.doc().root();
    let mut spec: AnimationSpec = manifest.default_spec();
    spec.preset = preset.clone();
    let reqs = [
        EditRequest::SetTextContent {
            seq,
            clip,
            content: RichText::new(SAMPLE_TEXT),
        },
        EditRequest::SetTextStyle {
            seq,
            clips: vec![clip],
            edit: TextStyleEdit::Size(RefPx::from_px(260)),
        },
        EditRequest::SetClipAnimation {
            seq,
            clips: vec![clip],
            slot,
            spec: Some(spec.clone()),
        },
    ];
    for req in &reqs {
        project.apply(req, Coalesce::Never).ok()?;
    }
    let stored = project
        .doc()
        .clip(clip)?
        .animations
        .get(slot)
        .map_or(spec.duration, |s| s.duration);
    Some((
        Arc::new(project.doc().clone()),
        tile_times(&manifest, slot, stored),
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use std::time::{Duration, Instant};

    use super::*;

    fn ms(n: i64) -> Tick {
        Tick::from_raw(TICKS_PER_SECOND / 1000 * n)
    }

    #[test]
    fn tiles_show_the_key_pose_and_a_short_loop() {
        let table = PresetTable::samples();
        let fade = table.get(&PresetId::new("fc.in.fade")).unwrap();
        let t = tile_times(&fade, Slot::Entrance, ms(500));
        assert_eq!(t.still, ms(300), "60 % of the motion");
        assert_eq!(t.start, Tick::ZERO);
        assert_eq!(t.frames, 12, "one second at 12 fps");
        assert_eq!(t.time_of(0), ms(300));
        assert_eq!(t.time_of(1), Tick::ZERO);
        assert_eq!(t.time_of(13), Tick::ZERO, "the loop wraps");
        let out = tile_times(&fade, Slot::Exit, ms(500));
        assert_eq!(out.still, ms(2_800));
        assert_eq!(out.start, ms(2_000));
        let wave = table.get(&PresetId::new("fc.loop.wave")).unwrap();
        let w = tile_times(&wave, Slot::Continuous, ms(1_200));
        assert_eq!((w.still, w.frames), (ms(720), 15));
        let long = tile_times(&wave, Slot::Continuous, ms(10_000));
        assert!(long.frames <= MAX_FRAMES);
    }

    /// The loop the Motion lens counts for a hovered tile is the one the
    /// worker renders, for every bundled preset in every slot: paced
    /// presets included, whatever the clip's own choice is.
    #[test]
    fn the_ui_counts_the_loop_the_worker_renders() {
        let table = freecut_project::presets::bundled().table;
        let mut checked = 0;
        for slot in Slot::ALL {
            for text in [true, false] {
                for c in crate::anim::slot_presets(&table, slot, text) {
                    let (_, worker) = sample(&c.id, slot, &table).unwrap();
                    assert_eq!(sample_times(&c.manifest, slot), worker, "{}", c.id);
                    checked += 1;
                }
            }
        }
        assert!(checked > 50, "{checked}");
    }

    fn lit(image: &PosterImage) -> usize {
        image
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] > 0 && p[0].max(p[1]).max(p[2]) > 128)
            .count()
    }

    /// The worker renders stills and a hovered tile's frames off the UI
    /// thread; a fade's first frame is empty and its still is not.
    #[test]
    fn the_worker_renders_stills_and_hover_frames() {
        let table = Arc::new(PresetTable::samples());
        let worker = PosterWorker::new(FontRegistry::bundled(), table, || {});
        let key = |frame| PosterKey {
            preset: PresetId::new("fc.in.fade"),
            slot: Slot::Entrance,
            frame,
        };
        worker.want_still(key(0));
        worker.want_motion(vec![key(1), key(12)]);
        let mut got: HashMap<u16, PosterImage> = HashMap::new();
        let deadline = Instant::now() + Duration::from_secs(60);
        while got.len() < 3 {
            for image in worker.poll() {
                got.insert(image.key.frame, image);
            }
            assert!(Instant::now() < deadline, "posters timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
        let still = &got[&0];
        assert_eq!((still.width, still.height), POSTER_SIZE);
        assert!(lit(still) > 50, "the still shows the text: {}", lit(still));
        assert_eq!(lit(&got[&1]), 0, "a fade starts invisible");
        assert!(lit(&got[&12]) >= lit(still), "and ends shown");
        // An unknown preset renders nothing and doesn't stop the worker.
        worker.want_still(PosterKey {
            preset: PresetId::new("fc.in.nothing"),
            slot: Slot::Entrance,
            frame: 0,
        });
        worker.want_still(PosterKey {
            preset: PresetId::new("fc.loop.wave"),
            slot: Slot::Continuous,
            frame: 0,
        });
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(image) = worker.poll().into_iter().next() {
                assert_eq!(image.key.preset, PresetId::new("fc.loop.wave"));
                break;
            }
            assert!(Instant::now() < deadline, "posters timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
