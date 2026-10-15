//! Filmstrip frames for the timeline's video and picture clips (ui-concept
//! §13.26, R5).
//!
//! The timeline asks for one frame per tile it paints, by [`FilmKey`]: the
//! library item, the frame height in pixels ([`height_bucket`]) and the
//! source time rounded down to a time bucket ([`bucket_time`]) as long as
//! the tile's span, from a ladder of powers of two so the buckets of one
//! zoom nest in the next and zooming reuses frames. Frames are decoded by
//! `freecut_media` on a thread of its own, newest request first, never on
//! the UI thread; [`Filmstrips::poll`] turns finished ones into textures.
//! Until a tile's own frame arrives the nearest frame of the same item
//! stands in, else the tile shows a placeholder. The textures are kept
//! within a memory budget, least recently painted first out.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use crossbeam_channel::{Receiver, Sender, unbounded};
use eframe::egui::{self, ColorImage, TextureHandle, TextureId, TextureOptions, Vec2};
use freecut_timeline::ids::MediaId;
use freecut_timeline::model::{MediaItem, Rotation};
use freecut_timeline::time::{TICKS_PER_SECOND, Tick};

/// Bytes of frame textures kept before the least recently painted go.
pub const BUDGET: usize = 64 * 1024 * 1024;
/// Requests waiting beyond this are dropped, oldest first (they are asked
/// for again if their tile is still on screen).
const MAX_QUEUED: usize = 96;
/// Frames turned into textures per UI frame.
const UPLOADS_PER_FRAME: usize = 24;
/// Buckets up to this long are decoded exactly, so files with long gaps
/// between keyframes don't show one keyframe on every tile; longer ones
/// take the nearest keyframe (fast).
const EXACT_UP_TO: Tick = Tick::from_raw(2 * TICKS_PER_SECOND);

/// One filmstrip frame: a library item, at a frame height, at a bucketed
/// source time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FilmKey {
    pub media: MediaId,
    pub height_px: u32,
    pub at: Tick,
}

/// The source time a tile spanning `span` of source time at `t` shows: `t`
/// rounded down to the bucket from 1/8 s, 1/4 s … 1024 s that is at least
/// `span` long. Pictures always show time 0.
pub fn bucket_time(t: Tick, span: Tick) -> Tick {
    let step = bucket_step(span);
    let t = t.max(Tick::ZERO);
    Tick::from_raw(t.raw() / step.raw() * step.raw())
}

/// The bucket length for a tile spanning `span`.
pub fn bucket_step(span: Tick) -> Tick {
    let mut step = TICKS_PER_SECOND / 8;
    let longest = 1_024 * TICKS_PER_SECOND;
    while step < span.raw() && step < longest {
        step *= 2;
    }
    Tick::from_raw(step)
}

/// The decoded height for a frame area `height_pt` tall at
/// `pixels_per_point`: rounded up to 8 px, within 16..=512.
pub fn height_bucket(height_pt: f32, pixels_per_point: f32) -> u32 {
    let px = (height_pt * pixels_per_point).ceil().clamp(16.0, 512.0) as u32;
    px.div_ceil(8) * 8
}

/// Where a frame comes from.
#[derive(Debug, Clone, PartialEq)]
pub struct FilmSource {
    pub path: PathBuf,
    /// The container index of the video stream.
    pub stream: usize,
    /// A picture: decoded once with its orientation applied.
    pub still: bool,
    pub rotation: Rotation,
    pub mirrored: bool,
    /// Width over height as displayed (square pixels, rotation applied).
    pub aspect: f32,
}

impl FilmSource {
    /// The default video stream of `item` (or its picture), read from
    /// `path`. `None` for sound-only items.
    pub fn of(item: &MediaItem, path: PathBuf) -> Option<FilmSource> {
        let (aspect, rotation, mirrored, stream) = geometry(item)?;
        Some(FilmSource {
            path,
            stream: usize::try_from(stream).ok()?,
            still: item.info.is_still(),
            rotation,
            mirrored,
            aspect,
        })
    }
}

/// Width over height of `item`'s picture as displayed (square pixels,
/// rotation applied); `None` for sound-only items.
pub fn display_aspect(item: &MediaItem) -> Option<f32> {
    geometry(item).map(|g| g.0)
}

/// The displayed aspect, rotation, mirroring and stream index of `item`'s
/// default video stream, or of its picture.
fn geometry(item: &MediaItem) -> Option<(f32, Rotation, bool, u32)> {
    let info = &item.info;
    let v = info
        .default_video
        .and_then(|i| info.video.iter().find(|v| v.index == i))
        .or_else(|| info.video.first());
    let (w, h, rotation, mirrored, stream) = match (v, info.still) {
        (Some(v), _) => {
            let (n, d) = v.sar;
            let w = if n > 0 && d > 0 {
                f64::from(v.width) * f64::from(n) / f64::from(d)
            } else {
                f64::from(v.width)
            };
            (w, f64::from(v.height), v.rotation, v.mirrored, v.index)
        }
        (None, Some((w, h))) => (f64::from(w), f64::from(h), Rotation::None, false, 0),
        (None, None) => return None,
    };
    let (w, h) = if quarter_turns(rotation) % 2 == 1 {
        (h, w)
    } else {
        (w, h)
    };
    let aspect = if h > 0.0 && w > 0.0 {
        (w / h) as f32
    } else {
        16.0 / 9.0
    };
    Some((aspect, rotation, mirrored, stream))
}

/// One frame to decode.
#[derive(Debug, Clone)]
pub struct FilmRequest {
    pub key: FilmKey,
    pub source: FilmSource,
    /// Decode the exact frame rather than the nearest keyframe.
    pub exact: bool,
    generation: u64,
}

/// A decoded frame: straight RGBA, rows top to bottom, as displayed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// What decodes frames on the worker thread (FFmpeg in the app, a fake in
/// tests). Built on that thread, so it needn't be `Send`.
pub trait FrameDecoder {
    fn decode(&mut self, request: &FilmRequest) -> Result<FrameImage, String>;
}

/// Builds the decoder on the worker thread.
pub type DecoderFactory = Box<dyn FnOnce() -> Box<dyn FrameDecoder> + Send>;

#[derive(Default)]
struct Queue {
    /// Newest first.
    items: VecDeque<FilmRequest>,
    stop: bool,
}

type Shared = Arc<(Mutex<Queue>, Condvar)>;

struct Done {
    generation: u64,
    key: FilmKey,
    result: Result<FrameImage, String>,
}

enum Entry {
    Pending,
    Ready {
        texture: TextureHandle,
        size: Vec2,
        bytes: usize,
        /// The UI frame it was last painted in.
        used: u64,
    },
    Failed,
}

/// The filmstrip frame cache and its worker.
pub struct Filmstrips {
    entries: HashMap<FilmKey, Entry>,
    /// Keys of ready frames, for the nearest stand-in.
    ready: BTreeSet<FilmKey>,
    queue: Shared,
    results: Receiver<Done>,
    thread: Option<JoinHandle<()>>,
    /// Requests and results from before the last [`Filmstrips::clear`]
    /// are dropped.
    generation: Arc<AtomicU64>,
    /// Counts [`Filmstrips::poll`] calls: one per UI frame.
    frame: u64,
    bytes: usize,
    budget: usize,
}

impl std::fmt::Debug for Filmstrips {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Filmstrips")
            .field("frames", &self.entries.len())
            .field("bytes", &self.bytes)
            .finish_non_exhaustive()
    }
}

impl Filmstrips {
    /// Starts the worker with FFmpeg's decoder; `wake` is called after
    /// each frame.
    pub fn new(wake: impl Fn() + Send + 'static) -> Filmstrips {
        Filmstrips::with_decoder(
            Box::new(|| Box::new(MediaFrames::default()) as Box<dyn FrameDecoder>),
            if freecut_media::resources::current().low_memory {
                8 << 20
            } else {
                BUDGET
            },
            wake,
        )
    }

    /// Starts the worker with the decoder `factory` builds, keeping at most
    /// `budget` bytes of textures.
    pub fn with_decoder(
        factory: DecoderFactory,
        budget: usize,
        wake: impl Fn() + Send + 'static,
    ) -> Filmstrips {
        let queue: Shared = Arc::default();
        let (tx, rx) = unbounded();
        let generation = Arc::new(AtomicU64::new(0));
        let shared = Arc::clone(&queue);
        let current = Arc::clone(&generation);
        let thread = std::thread::Builder::new()
            .name("freecut-filmstrips".into())
            .spawn(move || run(&shared, &current, &tx, factory, wake))
            .ok();
        Filmstrips {
            entries: HashMap::new(),
            ready: BTreeSet::new(),
            queue,
            results: rx,
            thread,
            generation,
            frame: 0,
            bytes: 0,
            budget,
        }
    }

    /// Forgets every frame and drops queued requests (another project).
    pub fn clear(&mut self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut q) = self.queue.0.lock() {
            q.items.clear();
        }
        self.entries.clear();
        self.ready.clear();
        self.bytes = 0;
    }

    /// Turns finished frames into textures and keeps the cache within its
    /// budget. Call once per UI frame.
    pub fn poll(&mut self, ctx: &egui::Context) {
        self.frame += 1;
        let current = self.generation.load(Ordering::SeqCst);
        for done in self.results.try_iter().take(UPLOADS_PER_FRAME) {
            if done.generation != current || !self.entries.contains_key(&done.key) {
                continue;
            }
            let entry = match done.result {
                Ok(image) => {
                    let size = [image.width as usize, image.height as usize];
                    let bytes = image.rgba.len();
                    let color = ColorImage::from_rgba_unmultiplied(size, &image.rgba);
                    let k = done.key;
                    let name = format!("film-{}-{}-{}", k.media.get(), k.height_px, k.at.raw());
                    let texture = ctx.load_texture(name, color, TextureOptions::LINEAR);
                    self.bytes += bytes;
                    self.ready.insert(done.key);
                    Entry::Ready {
                        texture,
                        size: Vec2::new(image.width as f32, image.height as f32),
                        bytes,
                        used: self.frame,
                    }
                }
                // The tile keeps its placeholder.
                Err(_) => Entry::Failed,
            };
            self.entries.insert(done.key, entry);
        }
        if !self.results.is_empty() {
            ctx.request_repaint();
        }
        self.evict();
    }

    /// The texture to paint for `key` and its pixel size: its own frame
    /// when ready, else the nearest ready frame of the same item and
    /// height; `None` for a placeholder. Asks for the frame when it hasn't
    /// been asked for (`source` says where it comes from; `span` is the
    /// tile's bucket length, for the decode mode).
    pub fn frame(
        &mut self,
        key: FilmKey,
        span: Tick,
        source: impl FnOnce() -> Option<FilmSource>,
    ) -> Option<(TextureId, Vec2)> {
        let frame = self.frame;
        match self.entries.get_mut(&key) {
            Some(Entry::Ready {
                texture,
                size,
                used,
                ..
            }) => {
                *used = frame;
                return Some((texture.id(), *size));
            }
            Some(_) => {}
            None => {
                if let Some(source) = source() {
                    self.request(key, source, span <= EXACT_UP_TO);
                } else {
                    self.entries.insert(key, Entry::Failed);
                }
            }
        }
        self.nearest(key)
    }

    /// Whether any frame asked for is still being decoded.
    #[cfg(test)]
    pub fn pending(&self) -> bool {
        self.entries.values().any(|e| matches!(e, Entry::Pending))
    }

    /// The ready frame of `key`'s item and height closest in time.
    fn nearest(&mut self, key: FilmKey) -> Option<(TextureId, Vec2)> {
        let low = FilmKey {
            at: Tick::from_raw(i64::MIN),
            ..key
        };
        let high = FilmKey {
            at: Tick::MAX,
            ..key
        };
        let before = self.ready.range(low..=key).next_back().copied();
        let after = self.ready.range(key..=high).next().copied();
        let distance = |k: &FilmKey| (k.at.raw() - key.at.raw()).unsigned_abs();
        let best = match (before, after) {
            (Some(a), Some(b)) => Some(if distance(&a) <= distance(&b) { a } else { b }),
            (a, b) => a.or(b),
        }?;
        let frame = self.frame;
        match self.entries.get_mut(&best) {
            Some(Entry::Ready {
                texture,
                size,
                used,
                ..
            }) => {
                *used = frame;
                Some((texture.id(), *size))
            }
            _ => None,
        }
    }

    fn request(&mut self, key: FilmKey, source: FilmSource, exact: bool) {
        self.entries.insert(key, Entry::Pending);
        let generation = self.generation.load(Ordering::SeqCst);
        let dropped: Vec<FilmKey> = {
            let (lock, wake) = &*self.queue;
            let Ok(mut q) = lock.lock() else { return };
            q.items.push_front(FilmRequest {
                key,
                source,
                exact,
                generation,
            });
            let mut dropped = Vec::new();
            while q.items.len() > MAX_QUEUED {
                if let Some(old) = q.items.pop_back() {
                    dropped.push(old.key);
                }
            }
            wake.notify_one();
            dropped
        };
        // Asked for again if their tiles are still on screen.
        for k in dropped {
            if matches!(self.entries.get(&k), Some(Entry::Pending)) {
                self.entries.remove(&k);
            }
        }
    }

    /// Drops the least recently painted frames (not painted this frame or
    /// the last) until the textures fit in 80 % of the budget.
    fn evict(&mut self) {
        if self.bytes <= self.budget {
            return;
        }
        let recent = self.frame.saturating_sub(1);
        let mut old: Vec<(u64, FilmKey)> = self
            .entries
            .iter()
            .filter_map(|(k, e)| match e {
                Entry::Ready { used, .. } if *used < recent => Some((*used, *k)),
                _ => None,
            })
            .collect();
        old.sort_unstable();
        let target = self.budget / 10 * 8;
        for (_, key) in old {
            if self.bytes <= target {
                break;
            }
            if let Some(Entry::Ready { bytes, .. }) = self.entries.remove(&key) {
                self.bytes -= bytes;
                self.ready.remove(&key);
            }
        }
    }
}

impl Drop for Filmstrips {
    fn drop(&mut self) {
        let (lock, wake) = &*self.queue;
        if let Ok(mut q) = lock.lock() {
            q.stop = true;
            q.items.clear();
            wake.notify_one();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(
    queue: &Shared,
    generation: &AtomicU64,
    results: &Sender<Done>,
    factory: DecoderFactory,
    wake: impl Fn(),
) {
    let mut decoder = factory();
    loop {
        // The newest request, then the others queued for the same item
        // and height in time order, so the decoder mostly moves forward.
        let batch = {
            let (lock, cvar) = &**queue;
            let Ok(mut q) = lock.lock() else { return };
            loop {
                if q.stop {
                    return;
                }
                if let Some(first) = q.items.pop_front() {
                    let mut batch = vec![first];
                    let (media, height) = (batch[0].key.media, batch[0].key.height_px);
                    let mut rest = VecDeque::new();
                    while let Some(r) = q.items.pop_front() {
                        if r.key.media == media && r.key.height_px == height {
                            batch.push(r);
                        } else {
                            rest.push_back(r);
                        }
                    }
                    q.items = rest;
                    batch.sort_by_key(|r| r.key.at);
                    break batch;
                }
                q = match cvar.wait(q) {
                    Ok(q) => q,
                    Err(_) => return,
                };
            }
        };
        for request in batch {
            if request.generation != generation.load(Ordering::SeqCst) {
                continue;
            }
            match queue.0.lock() {
                Ok(q) if !q.stop => {}
                _ => return,
            }
            let result = decoder.decode(&request);
            let done = Done {
                generation: request.generation,
                key: request.key,
                result,
            };
            if results.send(done).is_err() {
                return;
            }
            wake();
        }
    }
}

/// Quarter turns clockwise of `rotation`.
fn quarter_turns(rotation: Rotation) -> u8 {
    match rotation {
        Rotation::None => 0,
        Rotation::Cw90 => 1,
        Rotation::Cw180 => 2,
        Rotation::Cw270 => 3,
    }
}

/// `rgba` (`width × height`) mirrored when `mirrored`, then turned
/// clockwise by `rotation`: the picture as displayed.
pub fn orient(
    rgba: &[u8],
    width: u32,
    height: u32,
    rotation: Rotation,
    mirrored: bool,
) -> FrameImage {
    let (w, h) = (width as usize, height as usize);
    let turns = quarter_turns(rotation);
    if (turns == 0 && !mirrored) || rgba.len() < w * h * 4 {
        return FrameImage {
            width,
            height,
            rgba: rgba.to_vec(),
        };
    }
    let (dw, dh) = if turns % 2 == 1 { (h, w) } else { (w, h) };
    let mut out = vec![0u8; dw * dh * 4];
    for y in 0..h {
        for x in 0..w {
            let sx = if mirrored { w - 1 - x } else { x };
            let (tx, ty) = match turns {
                1 => (h - 1 - y, sx),
                2 => (w - 1 - sx, h - 1 - y),
                3 => (y, w - 1 - sx),
                _ => (sx, y),
            };
            let from = (y * w + x) * 4;
            let to = (ty * dw + tx) * 4;
            out[to..to + 4].copy_from_slice(&rgba[from..from + 4]);
        }
    }
    FrameImage {
        width: dw as u32,
        height: dh as u32,
        rgba: out,
    }
}

/// FFmpeg frames through `freecut_media`, keeping a few decoders open.
#[derive(Default)]
struct MediaFrames {
    /// Most recently used last.
    decoders: Vec<((PathBuf, usize), freecut_media::VideoDecoder)>,
}

/// Decoders kept open at once.
const OPEN_DECODERS: usize = 3;

impl FrameDecoder for MediaFrames {
    fn decode(&mut self, request: &FilmRequest) -> Result<FrameImage, String> {
        let s = &request.source;
        let height = request.key.height_px.max(1);
        let width = ((height as f32 * s.aspect).round() as u32).max(1);
        if s.still {
            // Pictures carry their own orientation (EXIF): the poster applies it.
            let side = width.max(height);
            let image = freecut_media::poster(&s.path, side).map_err(|e| e.to_string())?;
            return Ok(FrameImage {
                width: image.width,
                height: image.height,
                rgba: image.rgba,
            });
        }
        // The decoder works in coded orientation.
        let (cw, ch) = if quarter_turns(s.rotation) % 2 == 1 {
            (height, width)
        } else {
            (width, height)
        };
        let slot = (s.path.clone(), s.stream);
        let at = match self.decoders.iter().position(|(k, _)| *k == slot) {
            Some(i) => {
                let entry = self.decoders.remove(i);
                self.decoders.push(entry);
                self.decoders.len() - 1
            }
            None => {
                let decoder = freecut_media::VideoDecoder::open(&s.path, s.stream)
                    .map_err(|e| e.to_string())?;
                if self.decoders.len() >= OPEN_DECODERS {
                    self.decoders.remove(0);
                }
                self.decoders.push((slot, decoder));
                self.decoders.len() - 1
            }
        };
        let mode = if request.exact {
            freecut_media::SeekMode::Exact
        } else {
            freecut_media::SeekMode::Approximate
        };
        let decoder = &mut self.decoders[at].1;
        let frame = decoder
            .frame_at_size(request.key.at, mode, cw, ch)
            .map_err(|e| e.to_string())?;
        Ok(orient(
            &frame.rgba,
            frame.width,
            frame.height,
            s.rotation,
            s.mirrored,
        ))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use std::sync::atomic::AtomicUsize;
    use std::time::{Duration, Instant};

    use super::*;

    fn secs(n: f64) -> Tick {
        Tick::from_raw((n * TICKS_PER_SECOND as f64).round() as i64)
    }

    #[test]
    fn buckets_nest_and_grow_with_the_tile_span() {
        assert_eq!(bucket_step(Tick::ZERO), secs(0.125));
        assert_eq!(bucket_step(secs(0.3)), secs(0.5));
        assert_eq!(bucket_step(secs(1.0)), secs(1.0));
        assert_eq!(bucket_step(secs(1.5)), secs(2.0));
        assert_eq!(bucket_step(secs(1.0e6)), secs(1_024.0));
        assert_eq!(bucket_time(secs(3.7), secs(1.0)), secs(3.0));
        assert_eq!(bucket_time(secs(3.7), secs(2.0)), secs(2.0));
        assert_eq!(bucket_time(secs(-1.0), secs(1.0)), Tick::ZERO);
        // A coarse bucket's time is also a finer bucket's: zooming in
        // reuses the frame.
        for t in [0.0, 1.3, 7.9, 100.2] {
            let coarse = bucket_time(secs(t), secs(8.0));
            assert_eq!(bucket_time(coarse, secs(0.5)), coarse);
        }
        assert_eq!(height_bucket(45.0, 2.0), 96);
        assert_eq!(height_bucket(45.5, 1.0), 48);
        assert_eq!(height_bucket(2.0, 1.0), 16);
        assert_eq!(height_bucket(900.0, 2.0), 512);
    }

    #[test]
    fn orientation_turns_and_mirrors() {
        // 2 × 1: red, green.
        let px = [255, 0, 0, 255, 0, 255, 0, 255];
        let same = orient(&px, 2, 1, Rotation::None, false);
        assert_eq!(same.rgba, px);
        let turned = orient(&px, 2, 1, Rotation::Cw90, false);
        assert_eq!((turned.width, turned.height), (1, 2));
        assert_eq!(&turned.rgba[..4], &[255, 0, 0, 255], "red on top");
        let mirrored = orient(&px, 2, 1, Rotation::None, true);
        assert_eq!(&mirrored.rgba[..4], &[0, 255, 0, 255]);
        let half = orient(&px, 2, 1, Rotation::Cw180, false);
        assert_eq!(&half.rgba[..4], &[0, 255, 0, 255]);
        let back = orient(&px, 2, 1, Rotation::Cw270, false);
        assert_eq!(&back.rgba[..4], &[0, 255, 0, 255], "green on top");
    }

    /// Paints `frame` like the timeline would, polling between frames,
    /// until `done` or two seconds pass.
    fn wait(
        films: &mut Filmstrips,
        ctx: &egui::Context,
        mut done: impl FnMut(&mut Filmstrips) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !done(films) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
            films.poll(ctx);
        }
    }

    /// A decoder that paints each frame grey by its time, counting calls.
    struct Fake(Arc<AtomicUsize>);

    impl FrameDecoder for Fake {
        fn decode(&mut self, request: &FilmRequest) -> Result<FrameImage, String> {
            self.0.fetch_add(1, Ordering::SeqCst);
            if request.source.path.as_os_str() == "broken" {
                return Err("can't read".into());
            }
            let h = request.key.height_px;
            let w = (h as f32 * request.source.aspect) as u32;
            Ok(FrameImage {
                width: w,
                height: h,
                rgba: vec![128; (w * h * 4) as usize],
            })
        }
    }

    fn source(path: &str) -> FilmSource {
        FilmSource {
            path: path.into(),
            stream: 0,
            still: false,
            rotation: Rotation::None,
            mirrored: false,
            aspect: 2.0,
        }
    }

    fn fake(budget: usize) -> (Filmstrips, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let films = Filmstrips::with_decoder(
            Box::new(move || Box::new(Fake(counter)) as Box<dyn FrameDecoder>),
            budget,
            || {},
        );
        (films, calls)
    }

    fn media_id(n: u64) -> MediaId {
        MediaId::from_raw(std::num::NonZeroU64::new(n).unwrap())
    }

    fn key(media: MediaId, at: f64) -> FilmKey {
        FilmKey {
            media,
            height_px: 16,
            at: secs(at),
        }
    }

    #[test]
    fn frames_are_decoded_once_off_the_ui_thread_and_stand_in_for_neighbours() {
        let ctx = egui::Context::default();
        let (mut films, calls) = fake(BUDGET);
        let media = media_id(1);
        // Nothing yet: a placeholder, and one request.
        assert!(
            films
                .frame(key(media, 1.0), secs(1.0), || Some(source("a")))
                .is_none()
        );
        assert!(films.pending());
        wait(&mut films, &ctx, |f| !f.pending());
        let ready = films.frame(key(media, 1.0), secs(1.0), || None);
        assert_eq!(ready.map(|(_, s)| s), Some(Vec2::new(32.0, 16.0)));
        // Asked again: from the cache, not decoded again.
        films.frame(key(media, 1.0), secs(1.0), || Some(source("a")));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // A tile whose frame is still coming shows the nearest ready one.
        let near = films.frame(key(media, 3.0), secs(1.0), || Some(source("a")));
        assert_eq!(near.map(|(t, _)| t), ready.map(|(t, _)| t));
        // Another item has no stand-in.
        let other = media_id(2);
        assert!(
            films
                .frame(key(other, 1.0), secs(1.0), || Some(source("broken")))
                .is_none()
        );
        wait(&mut films, &ctx, |f| !f.pending());
        // A failed frame isn't asked for again.
        assert!(
            films
                .frame(key(other, 1.0), secs(1.0), || Some(source("broken")))
                .is_none()
        );
        assert!(!films.pending());
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        // Another project: everything goes.
        films.clear();
        assert!(films.frame(key(media, 1.0), secs(1.0), || None).is_none());
    }

    #[test]
    fn the_cache_keeps_to_its_budget_dropping_frames_not_painted_lately() {
        let ctx = egui::Context::default();
        // Each 32 × 16 frame is 2 KiB; room for three.
        let (mut films, _) = fake(3 * 2048);
        let media = media_id(1);
        for t in 0..6 {
            films.frame(key(media, f64::from(t)), secs(1.0), || Some(source("a")));
            wait(&mut films, &ctx, |f| !f.pending());
        }
        for _ in 0..3 {
            films.poll(&ctx);
        }
        assert!(films.bytes <= 3 * 2048, "{}", films.bytes);
        // The newest frames stayed.
        let last = key(media, 5.0);
        assert!(films.ready.contains(&last));
        assert!(!films.ready.contains(&key(media, 0.0)));
    }

    /// FFmpeg decodes three frames of a real clip on the worker thread.
    #[test]
    fn real_frames_arrive_from_the_worker() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clip.mp4");
        let made = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
            .arg("testsrc2=size=320x180:rate=30:duration=3")
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .arg(&path)
            .status()
            .is_ok_and(|s| s.success());
        if !made || freecut_media::init().is_err() {
            eprintln!("no ffmpeg CLI: skipped");
            return;
        }
        let ctx = egui::Context::default();
        let mut films = Filmstrips::new(|| {});
        let media = media_id(7);
        let source = FilmSource {
            path: path.clone(),
            stream: 0,
            still: false,
            rotation: Rotation::None,
            mirrored: false,
            aspect: 16.0 / 9.0,
        };
        let keys: Vec<FilmKey> = [0.0, 1.0, 2.0]
            .iter()
            .map(|t| FilmKey {
                media,
                height_px: 48,
                at: secs(*t),
            })
            .collect();
        for k in &keys {
            films.frame(*k, secs(1.0), || Some(source.clone()));
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        while films.pending() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
            films.poll(&ctx);
        }
        for k in &keys {
            let (_, size) = films.frame(*k, secs(1.0), || None).unwrap();
            assert_eq!(size.y, 48.0);
            assert!((size.x - 85.0).abs() <= 1.0, "{size:?}");
        }
    }
}
