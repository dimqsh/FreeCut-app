//! Sequences (TL-003).

use std::sync::Arc;

use super::audio::MasterAudio;
use super::follow::FollowKinds;
use super::marker::Markers;
use super::track::{Track, TrackKind};
use crate::ids::{ClipId, SeqId, TrackId};
use crate::time::Tick;

/// Undoable per-sequence behavior switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SequenceFlags {
    /// The magnetic main track (TL-005).
    pub main_magnet: bool,
    /// *Follow main track* (TL-029): items over main-track clips are
    /// attached to them and follow them.
    pub follow_main: bool,
    /// Which kinds of items follow (TL-029's *Item kinds…*).
    pub follow_kinds: FollowKinds,
}

impl Default for SequenceFlags {
    fn default() -> Self {
        SequenceFlags {
            main_magnet: true,
            follow_main: true,
            follow_kinds: FollowKinds::ALL,
        }
    }
}

impl SequenceFlags {
    /// Whether any kind of item follows main-track clips.
    pub const fn follows(self) -> bool {
        self.follow_main && !self.follow_kinds.is_empty()
    }
}

/// Which list a track lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Lane {
    /// Visual tracks, bottom to top. Index 0 is the main track.
    Visual,
    /// Audio tracks, top to bottom.
    Audio,
}

/// A track's position in its sequence. Orders visual slots (bottom-up)
/// before audio slots (top-down).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TrackSlot {
    pub lane: Lane,
    pub index: usize,
}

impl TrackSlot {
    pub const MAIN: TrackSlot = TrackSlot {
        lane: Lane::Visual,
        index: 0,
    };

    pub const fn visual(index: usize) -> TrackSlot {
        TrackSlot {
            lane: Lane::Visual,
            index,
        }
    }

    pub const fn audio(index: usize) -> TrackSlot {
        TrackSlot {
            lane: Lane::Audio,
            index,
        }
    }

    pub const fn is_main(self) -> bool {
        matches!(self.lane, Lane::Visual) && self.index == 0
    }
}

/// A timeline: stacked visual tracks over the main track, plus audio tracks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sequence {
    pub id: SeqId,
    pub flags: SequenceFlags,
    /// The master stage (AU-010). Changed only through undoable edits.
    pub master: MasterAudio,
    /// Timeline markers in sequence time, sorted (TL-037). Changed only
    /// through undoable edits; a loader sets them after
    /// [`Sequence::from_parts`].
    pub markers: Markers,
    visual: Vec<Arc<Track>>,
    audio: Vec<Arc<Track>>,
}

impl Sequence {
    /// A sequence holding only an empty main track.
    pub fn new(id: SeqId, main_track: TrackId) -> Sequence {
        Sequence {
            id,
            flags: SequenceFlags::default(),
            master: MasterAudio::default(),
            markers: Markers::default(),
            visual: vec![Arc::new(Track::new(main_track, TrackKind::Video))],
            audio: Vec::new(),
        }
    }

    /// Builds a sequence from stored parts (project loading). Invariants are
    /// checked by [`crate::model::Document::validate`]. The master stage
    /// starts at its default; a loader sets [`Sequence::master`] afterward.
    pub fn from_parts(
        id: SeqId,
        flags: SequenceFlags,
        visual: Vec<Arc<Track>>,
        audio: Vec<Arc<Track>>,
    ) -> Sequence {
        Sequence {
            id,
            flags,
            master: MasterAudio::default(),
            markers: Markers::default(),
            visual,
            audio,
        }
    }

    pub fn visual_tracks(&self) -> &[Arc<Track>] {
        &self.visual
    }

    pub fn audio_tracks(&self) -> &[Arc<Track>] {
        &self.audio
    }

    /// The main track, if the sequence is well formed.
    pub fn main_track(&self) -> Option<&Arc<Track>> {
        self.visual.first()
    }

    /// Every track, visual (bottom to top) then audio (top to bottom).
    pub fn tracks(&self) -> impl Iterator<Item = (TrackSlot, &Arc<Track>)> {
        let visual = self
            .visual
            .iter()
            .enumerate()
            .map(|(i, t)| (TrackSlot::visual(i), t));
        let audio = self
            .audio
            .iter()
            .enumerate()
            .map(|(i, t)| (TrackSlot::audio(i), t));
        visual.chain(audio)
    }

    pub fn track_count(&self) -> usize {
        self.visual.len() + self.audio.len()
    }

    pub fn lane(&self, lane: Lane) -> &[Arc<Track>] {
        match lane {
            Lane::Visual => &self.visual,
            Lane::Audio => &self.audio,
        }
    }

    pub fn track_at(&self, slot: TrackSlot) -> Option<&Arc<Track>> {
        self.lane(slot.lane).get(slot.index)
    }

    pub fn find_track(&self, id: TrackId) -> Option<(TrackSlot, &Arc<Track>)> {
        self.tracks().find(|(_, t)| t.id == id)
    }

    /// The track holding clip `id`.
    pub fn find_clip(&self, id: ClipId) -> Option<(TrackSlot, &Arc<Track>)> {
        self.tracks().find(|(_, t)| t.clip(id).is_some())
    }

    /// End of the last item on any track (TL-003). Hidden, muted and disabled
    /// items count, because they still occupy time.
    pub fn duration(&self) -> Tick {
        self.tracks()
            .map(|(_, t)| t.end())
            .max()
            .unwrap_or(Tick::ZERO)
    }

    pub(crate) fn lane_mut(&mut self, lane: Lane) -> &mut Vec<Arc<Track>> {
        match lane {
            Lane::Visual => &mut self.visual,
            Lane::Audio => &mut self.audio,
        }
    }

    pub(crate) fn track_mut(&mut self, id: TrackId) -> Option<&mut Track> {
        self.visual
            .iter_mut()
            .chain(self.audio.iter_mut())
            .find(|t| t.id == id)
            .map(Arc::make_mut)
    }
}
