//! A ready-made [`LayerImages`] backed by `freecut-media` decoders, shared
//! by preview and export.
//!
//! One decoder per asset is opened on first use and kept; an asset whose
//! file can't be opened is remembered as failed (it draws the slate) until
//! its source changes. Stills are decoded once. The last video frame of
//! each asset is kept, so asking again for a time it still covers (at the
//! same size) costs nothing. Video shown much smaller than its size is
//! converted at about the size it is drawn
//! ([`LayerImages::video_scaled`]), which is both cheaper and filtered for
//! the downscale. Open and decode failures are recorded for
//! [`LayerImages::take_failures`] (export stops on them); the most recent
//! [`MAX_FAILURES`] are kept. Everything here blocks: call it from a render
//! thread.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use freecut_media::{SeekMode, VideoDecoder};
use freecut_timeline::ids::MediaId;
use freecut_timeline::model::{Library, MediaItem, Rotation};
use freecut_timeline::plan::{FramePlan, LayerSource, PlanNode};
use freecut_timeline::time::Tick;

use crate::cpu::{ImageFailure, LayerImages, Picture};

/// Failures kept for [`LayerImages::take_failures`] (the oldest are
/// dropped, so a preview that never takes them stays bounded).
pub const MAX_FAILURES: usize = 64;

/// An idle asset must not keep a decoder, its FFmpeg reference frames and
/// converted RGBA buffers alive for the whole editing session.
fn open_decoders() -> usize {
    freecut_media::resources::current().idle_decoders
}

/// Where an asset's pictures come from and how they are turned for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaFile {
    pub path: PathBuf,
    /// Container index of the video stream to decode.
    pub stream: usize,
    pub rotation: Rotation,
    pub mirrored: bool,
}

impl MediaFile {
    /// The file of `item` at `path`, showing its default video stream;
    /// `None` when the item has no video.
    pub fn of_item(item: &MediaItem, path: PathBuf) -> Option<MediaFile> {
        let info = &item.info;
        let stream = info
            .default_video
            .or_else(|| info.video.first().map(|v| v.index))?;
        let video = info.video.iter().find(|v| v.index == stream);
        Some(MediaFile {
            path,
            stream: stream as usize,
            rotation: video.map(|v| v.rotation).unwrap_or_default(),
            mirrored: video.is_some_and(|v| v.mirrored),
        })
    }

    /// The files of every library item that has a path in `paths`.
    pub fn from_library(
        library: &Library,
        paths: &BTreeMap<MediaId, PathBuf>,
    ) -> BTreeMap<MediaId, MediaFile> {
        paths
            .iter()
            .filter_map(|(id, path)| {
                let item = library.get(*id)?;
                Some((*id, MediaFile::of_item(item, path.clone())?))
            })
            .collect()
    }
}

enum Slot {
    Ready {
        file: MediaFile,
        decoder: Box<VideoDecoder>,
        /// The last frame returned, with the media time range it covers.
        last: Option<LastFrame>,
        still: Option<Arc<Picture>>,
    },
    /// Opening failed; draw a slate rather than retrying every frame.
    Failed { file: MediaFile, reason: String },
}

struct LastFrame {
    start: Tick,
    duration: Option<Tick>,
    /// The requested size; `None` for the coded size.
    size: Option<(u32, u32)>,
    picture: Arc<Picture>,
}

impl Slot {
    fn file(&self) -> &MediaFile {
        match self {
            Slot::Ready { file, .. } | Slot::Failed { file, .. } => file,
        }
    }
}

/// Decoded pictures for the compositor, by asset.
pub struct MediaImages {
    files: BTreeMap<MediaId, MediaFile>,
    slots: HashMap<MediaId, Slot>,
    mode: SeekMode,
    failures: Vec<ImageFailure>,
    /// Open decoders, most recently requested last. Failed opens have no
    /// decoder and stay remembered separately until their file changes.
    recent: VecDeque<MediaId>,
    active: BTreeSet<MediaId>,
}

impl std::fmt::Debug for MediaImages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaImages")
            .field("files", &self.files.len())
            .field("open", &self.slots.len())
            .field("mode", &self.mode)
            .field("failures", &self.failures.len())
            .finish()
    }
}

impl Default for MediaImages {
    fn default() -> Self {
        MediaImages::new()
    }
}

impl MediaImages {
    /// No files yet; frames are exact.
    pub fn new() -> MediaImages {
        MediaImages {
            files: BTreeMap::new(),
            slots: HashMap::new(),
            mode: SeekMode::Exact,
            failures: Vec::new(),
            recent: VecDeque::new(),
            active: BTreeSet::new(),
        }
    }

    /// Replaces the asset → file map. Decoders of assets that disappeared
    /// or whose file changed are closed; the others are kept.
    pub fn set_files(&mut self, files: BTreeMap<MediaId, MediaFile>) {
        self.slots
            .retain(|id, slot| files.get(id).is_some_and(|f| f == slot.file()));
        self.recent.retain(|id| self.slots.contains_key(id));
        self.files = files;
    }

    /// Exact (export, paused preview) or approximate (fast scrubbing).
    pub fn set_seek_mode(&mut self, mode: SeekMode) {
        self.mode = mode;
    }

    fn slot(&mut self, asset: MediaId) -> Option<&mut Slot> {
        if !self.slots.contains_key(&asset) {
            let file = self.files.get(&asset)?.clone();
            if self.recent.len() >= open_decoders().max(self.active.len()) {
                self.evict_idle();
            }
            let slot = match VideoDecoder::open(&file.path, file.stream) {
                Ok(decoder) => Slot::Ready {
                    file,
                    decoder: Box::new(decoder),
                    last: None,
                    still: None,
                },
                Err(e) => {
                    tracing::warn!("can't open {}: {e}", file.path.display());
                    Slot::Failed {
                        file,
                        reason: e.to_string(),
                    }
                }
            };
            self.slots.insert(asset, slot);
        }
        if matches!(self.slots.get(&asset), Some(Slot::Ready { .. })) {
            self.recent.retain(|id| *id != asset);
            self.recent.push_back(asset);
        }
        self.slots.get_mut(&asset)
    }

    /// Drops one least-recent decoder that the current frame doesn't need.
    fn evict_idle(&mut self) -> bool {
        let Some(i) = self.recent.iter().position(|id| !self.active.contains(id)) else {
            return false;
        };
        if let Some(id) = self.recent.remove(i) {
            self.slots.remove(&id);
        }
        true
    }
}

impl MediaImages {
    fn fail(&mut self, asset: MediaId, source_time: Option<Tick>, reason: String) {
        if self.failures.len() >= MAX_FAILURES {
            self.failures.remove(0);
        }
        self.failures.push(ImageFailure {
            asset,
            source_time,
            reason,
        });
    }

    /// The frame of `asset` at `source_time`, converted at `size` (`None`:
    /// the coded size).
    fn frame(
        &mut self,
        asset: MediaId,
        source_time: Tick,
        size: impl FnOnce((u32, u32), Rotation) -> Option<(u32, u32)>,
    ) -> Option<Arc<Picture>> {
        let mode = self.mode;
        let result = match self.slot(asset)? {
            Slot::Failed { reason, .. } => Err(reason.clone()),
            Slot::Ready {
                file,
                decoder,
                last,
                ..
            } => {
                let size = size(decoder.coded_size(), file.rotation);
                if let Some(l) = last.as_ref()
                    && mode == SeekMode::Exact
                    && l.size == size
                    && l.start <= source_time
                    && l.duration
                        .is_some_and(|d| l.start.checked_add(d).is_ok_and(|end| source_time < end))
                {
                    return Some(Arc::clone(&l.picture));
                }
                let frame = match size {
                    Some((w, h)) => decoder.frame_at_size(source_time, mode, w, h),
                    None => decoder.frame_at(source_time, mode),
                };
                frame
                    .map(|frame| {
                        let picture = Arc::new(Picture {
                            width: frame.width,
                            height: frame.height,
                            rotation: file.rotation,
                            mirrored: file.mirrored,
                            rgba: frame.rgba,
                        });
                        *last = Some(LastFrame {
                            start: frame.pts,
                            duration: frame.duration,
                            size,
                            picture: Arc::clone(&picture),
                        });
                        picture
                    })
                    .map_err(|e| e.to_string())
            }
        };
        match result {
            Ok(picture) => Some(picture),
            Err(reason) => {
                tracing::debug!("no picture for {asset} at {source_time}: {reason}");
                self.fail(asset, Some(source_time), reason);
                None
            }
        }
    }
}

/// The stored size to convert a `coded` picture at, for a layer drawn at
/// `fraction` of its display size (display axes; swapped for a quarter
/// turn): at least that large, never larger than coded, `None` when that
/// is the coded size anyway.
fn scaled_size(coded: (u32, u32), rotation: Rotation, fraction: (f64, f64)) -> Option<(u32, u32)> {
    let (fx, fy) = match rotation {
        Rotation::Cw90 | Rotation::Cw270 => (fraction.1, fraction.0),
        Rotation::None | Rotation::Cw180 => fraction,
    };
    let side = |n: u32, f: f64| -> u32 {
        if !f.is_finite() || f >= 1.0 {
            return n;
        }
        // Integer-valued and within 1..=n, so the cast is exact.
        (f64::from(n) * f.max(0.0)).ceil().clamp(1.0, f64::from(n)) as u32
    };
    let size = (side(coded.0, fx), side(coded.1, fy));
    (size != coded).then_some(size)
}

impl LayerImages for MediaImages {
    fn begin_frame(&mut self, plan: &FramePlan) {
        self.active = plan
            .nodes
            .iter()
            .filter_map(|node| match node {
                PlanNode::Layer(layer) => match layer.source {
                    LayerSource::Video { asset, .. } | LayerSource::Still { asset } => Some(asset),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        while self.recent.len() > open_decoders().max(self.active.len()) && self.evict_idle() {}
    }

    fn video(&mut self, asset: MediaId, source_time: Tick) -> Option<Arc<Picture>> {
        self.frame(asset, source_time, |_, _| None)
    }

    fn video_scaled(
        &mut self,
        asset: MediaId,
        source_time: Tick,
        fraction: (f64, f64),
    ) -> Option<Arc<Picture>> {
        self.frame(asset, source_time, |coded, rotation| {
            scaled_size(coded, rotation, fraction)
        })
    }

    fn still(&mut self, asset: MediaId) -> Option<Arc<Picture>> {
        let result = match self.slot(asset)? {
            Slot::Failed { reason, .. } => Err(reason.clone()),
            Slot::Ready {
                file,
                decoder,
                still,
                ..
            } => {
                if let Some(picture) = still {
                    return Some(Arc::clone(picture));
                }
                decoder
                    .frame_at(Tick::ZERO, SeekMode::Exact)
                    .map(|frame| {
                        let picture = Arc::new(Picture {
                            width: frame.width,
                            height: frame.height,
                            rotation: file.rotation,
                            mirrored: file.mirrored,
                            rgba: frame.rgba,
                        });
                        *still = Some(Arc::clone(&picture));
                        picture
                    })
                    .map_err(|e| e.to_string())
            }
        };
        match result {
            Ok(picture) => Some(picture),
            Err(reason) => {
                self.fail(asset, None, reason);
                None
            }
        }
    }

    fn take_failures(&mut self) -> Vec<ImageFailure> {
        std::mem::take(&mut self.failures)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::unwrap_used)] // test code
    fn idle_decoders_are_bounded_but_simultaneous_layers_stay_open() {
        use std::num::NonZeroU64;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("still.png");
        image::RgbaImage::from_pixel(16, 9, image::Rgba([20, 30, 40, 255]))
            .save(&path)
            .unwrap();
        let id = |i| MediaId::from_raw(NonZeroU64::new(i).unwrap());
        let files = (1..=12)
            .map(|i| {
                (
                    id(i),
                    MediaFile {
                        path: path.clone(),
                        stream: 0,
                        rotation: Rotation::None,
                        mirrored: false,
                    },
                )
            })
            .collect();
        let mut images = MediaImages::new();
        images.set_files(files);
        for i in 1..=12 {
            assert!(images.still(id(i)).is_some());
            assert!(images.recent.len() <= open_decoders());
        }
        assert!(!images.slots.contains_key(&id(1)));
        // More than four simultaneously visible sources must not thrash
        // their decoders on each frame.
        images.active = (1..=6).map(id).collect();
        for i in 1..=6 {
            assert!(images.still(id(i)).is_some());
        }
        let old: Vec<_> = (1..=6).map(|i| images.still(id(i)).unwrap()).collect();
        for (i, old) in (1..=6).zip(old) {
            assert!(Arc::ptr_eq(&old, &images.still(id(i)).unwrap()));
        }
        images.set_files(BTreeMap::new());
        assert!(images.slots.is_empty());
        assert!(images.recent.is_empty());
    }

    #[test]
    fn scaled_sizes_cover_the_drawn_size() {
        assert_eq!(
            scaled_size((1920, 1080), Rotation::None, (0.25, 0.25)),
            Some((480, 270))
        );
        assert_eq!(
            scaled_size((1920, 1080), Rotation::None, (0.3, 0.1)),
            Some((576, 108))
        );
        // A quarter turn swaps the display axes onto the stored ones.
        assert_eq!(
            scaled_size((1920, 1080), Rotation::Cw90, (0.3, 0.1)),
            Some((192, 324))
        );
        // Never larger than coded, never zero.
        assert_eq!(scaled_size((1920, 1080), Rotation::None, (1.5, 2.0)), None);
        assert_eq!(
            scaled_size((10, 10), Rotation::None, (0.0, 1e-9)),
            Some((1, 1))
        );
        assert_eq!(
            scaled_size((10, 10), Rotation::None, (f64::NAN, 0.5)),
            Some((10, 5))
        );
    }
}
