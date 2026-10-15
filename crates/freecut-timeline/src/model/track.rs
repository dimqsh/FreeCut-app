//! Tracks (TL-004).

use std::sync::Arc;

use super::audio::TrackAudio;
use super::clip::{Clip, ClipContent, GeneratedKind, MediaKind};
use crate::ids::{ClipId, TrackId};
use crate::time::{Tick, TimeRange};

/// What a track holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrackKind {
    Video,
    Text,
    Sticker,
    Effect,
    Adjustment,
    Audio,
}

/// Visual layering bands, bottom to top (TL-004). Audio tracks have no band.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Band {
    Video,
    Upper,
}

impl TrackKind {
    pub const fn is_audio(self) -> bool {
        matches!(self, TrackKind::Audio)
    }

    /// Whether clips on tracks of this kind can make sound: audio clips, and
    /// the embedded sound of video clips (AU-013).
    pub const fn carries_sound(self) -> bool {
        matches!(self, TrackKind::Audio | TrackKind::Video)
    }

    pub const fn band(self) -> Option<Band> {
        match self {
            TrackKind::Video => Some(Band::Video),
            TrackKind::Text | TrackKind::Sticker | TrackKind::Effect | TrackKind::Adjustment => {
                Some(Band::Upper)
            }
            TrackKind::Audio => None,
        }
    }

    /// Whether clips with this content may sit on a track of this kind.
    pub fn accepts(self, content: &ClipContent) -> bool {
        self == TrackKind::for_content(content)
    }

    /// The one track kind that holds this content.
    pub fn for_content(content: &ClipContent) -> TrackKind {
        match content {
            ClipContent::Media {
                kind: MediaKind::Video,
                ..
            }
            | ClipContent::Image { .. } => TrackKind::Video,
            ClipContent::Media {
                kind: MediaKind::Audio,
                ..
            } => TrackKind::Audio,
            ClipContent::Text(_) => TrackKind::Text,
            ClipContent::Generated { kind } => match kind {
                GeneratedKind::Sticker => TrackKind::Sticker,
                GeneratedKind::Effect => TrackKind::Effect,
                GeneratedKind::Adjustment => TrackKind::Adjustment,
            },
        }
    }
}

/// Per-track toggles. All of them are undoable (TL-008, TL-009).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct TrackFlags {
    pub muted: bool,
    pub hidden: bool,
    pub locked: bool,
}

/// The editable header of a track.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct TrackHeader {
    pub name: Option<String>,
    pub flags: TrackFlags,
}

/// One lane of non-overlapping clips, sorted by start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub id: TrackId,
    pub kind: TrackKind,
    pub header: TrackHeader,
    /// The track stage (AU-013): volume and pan. Defaults on tracks that
    /// carry no sound. Changed only through undoable edits; a loader sets it
    /// after [`Track::from_parts`].
    pub mix: TrackAudio,
    clips: Vec<Arc<Clip>>,
}

impl Track {
    pub fn new(id: TrackId, kind: TrackKind) -> Track {
        Track {
            id,
            kind,
            header: TrackHeader::default(),
            mix: TrackAudio::default(),
            clips: Vec::new(),
        }
    }

    /// Builds a track from stored parts (project loading). Clips are put in
    /// order; overlaps and other invariants are checked by
    /// [`crate::model::Document::validate`]. The track stage starts at its
    /// default; a loader sets [`Track::mix`] afterward.
    pub fn from_parts(
        id: TrackId,
        kind: TrackKind,
        header: TrackHeader,
        mut clips: Vec<Arc<Clip>>,
    ) -> Track {
        clips.sort_by_key(|c| (c.start(), c.id));
        Track {
            id,
            kind,
            header,
            mix: TrackAudio::default(),
            clips,
        }
    }

    pub fn clips(&self) -> &[Arc<Clip>] {
        &self.clips
    }

    pub fn is_empty(&self) -> bool {
        self.clips.is_empty()
    }

    pub fn is_locked(&self) -> bool {
        self.header.flags.locked
    }

    /// Position of the clip with this start and id: a binary search.
    pub fn index_of(&self, start: Tick, id: ClipId) -> Option<usize> {
        crate::probe::visited(1);
        self.clips
            .binary_search_by_key(&(start, id), |c| (c.start(), c.id))
            .ok()
    }

    /// Position of clip `id` by linear search. Prefer [`Track::index_of`] or
    /// [`crate::model::Document::clip`].
    pub fn position_of(&self, id: ClipId) -> Option<usize> {
        let found = self.clips.iter().position(|c| c.id == id);
        crate::probe::visited(found.map_or(self.clips.len(), |i| i + 1));
        found
    }

    /// Clip `id` by linear search. Prefer [`crate::model::Document::clip`].
    pub fn clip(&self, id: ClipId) -> Option<&Arc<Clip>> {
        self.position_of(id).map(|i| &self.clips[i])
    }

    /// End of the last clip, or 0 for an empty track.
    pub fn end(&self) -> Tick {
        // Clips never overlap, so the last one ends last.
        self.clips.last().map_or(Tick::ZERO, |c| c.end())
    }

    /// The clip covering `t`, if any.
    pub fn clip_at(&self, t: Tick) -> Option<&Arc<Clip>> {
        let i = self.clips.partition_point(|c| c.start() <= t);
        i.checked_sub(1)
            .map(|i| &self.clips[i])
            .filter(|c| c.range.contains(t))
    }

    /// Clips overlapping `range`, in order.
    pub fn clips_in_range(&self, range: TimeRange) -> impl Iterator<Item = &Arc<Clip>> {
        let first = self.clips.partition_point(|c| c.end() <= range.start());
        self.clips[first..]
            .iter()
            .take_while(move |c| c.start() < range.end())
    }

    /// Empty spans before each clip (the trailing space is never a gap).
    pub fn gaps(&self) -> impl Iterator<Item = (ClipId, TimeRange)> + '_ {
        let mut prev_end = Tick::ZERO;
        self.clips.iter().filter_map(move |c| {
            let gap = TimeRange::from_start_end(prev_end, c.start()).ok();
            prev_end = c.end();
            gap.map(|g| (c.id, g))
        })
    }

    pub(crate) fn insert(&mut self, clip: Arc<Clip>) {
        let at = self
            .clips
            .partition_point(|c| (c.start(), c.id) < (clip.start(), clip.id));
        self.clips.insert(at, clip);
    }

    pub(crate) fn remove_at(&mut self, index: usize) -> Arc<Clip> {
        self.clips.remove(index)
    }

    pub(crate) fn clips_mut(&mut self) -> &mut Vec<Arc<Clip>> {
        &mut self.clips
    }

    pub(crate) fn resort(&mut self) {
        // Stable and adaptive: nearly sorted input (a ripple) is linear.
        self.clips.sort_by_key(|c| (c.start(), c.id));
    }
}
