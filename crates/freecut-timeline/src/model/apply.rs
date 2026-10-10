//! Applying primitive changes to a document.
//!
//! `apply` checks structural preconditions only: the target exists or
//! doesn't, the recorded `before` state matches, and ids stay unique across
//! the document. It never consults locks, the magnet or limits, and it
//! doesn't check overlaps: a compound edit may pass through transient
//! states, and invariants are checked once at the end of the transaction.
//! It keeps the derived clip index in step with every change.

use std::collections::BTreeSet;
use std::sync::Arc;

use super::{Document, IndexEntry, Sequence};
use crate::edit::{ApplyError, Change};
use crate::ids::{ClipId, SeqId, TrackId};
use crate::model::{Clip, Track};
use crate::time::Tick;

impl Document {
    pub(crate) fn apply(&mut self, change: &Change) -> Result<(), ApplyError> {
        match change {
            Change::InsertClip { seq, track, clip } => {
                if self.index_entry(clip.id).is_some() {
                    return Err(ApplyError::ClipExists(clip.id));
                }
                self.track_mut(*seq, *track)?.insert(Arc::clone(clip));
                self.index_insert(*seq, *track, clip);
            }
            Change::RemoveClip { seq, track, clip } => {
                let t = self.located_track_mut(*seq, *track, clip.id)?;
                let i = t
                    .index_of(clip.start(), clip.id)
                    .ok_or(ApplyError::BeforeMismatch)?;
                if !same(&t.clips()[i], clip) {
                    return Err(ApplyError::BeforeMismatch);
                }
                t.remove_at(i);
                self.index_mut().remove(&clip.id);
            }
            Change::ReplaceClip {
                seq,
                track,
                before,
                after,
            } => {
                if before.id != after.id {
                    return Err(ApplyError::IdMismatch);
                }
                let t = self.located_track_mut(*seq, *track, before.id)?;
                let i = t
                    .index_of(before.start(), before.id)
                    .ok_or(ApplyError::BeforeMismatch)?;
                if !same(&t.clips()[i], before) {
                    return Err(ApplyError::BeforeMismatch);
                }
                t.remove_at(i);
                t.insert(Arc::clone(after));
                self.index_insert(*seq, *track, after);
            }
            Change::ShiftClips {
                seq,
                track,
                moves,
                delta,
            } => {
                let wanted: BTreeSet<(Tick, ClipId)> =
                    moves.iter().map(|&(id, s)| (s, id)).collect();
                if wanted.len() != moves.len() {
                    return Err(ApplyError::BeforeMismatch);
                }
                let t = self.track_mut(*seq, *track)?;
                let mut found = 0;
                for slot in t.clips_mut().iter_mut() {
                    if wanted.contains(&(slot.start(), slot.id)) {
                        let moved = slot.moved_to(slot.start().checked_add(*delta)?)?;
                        *slot = Arc::new(moved);
                        found += 1;
                    }
                }
                if found != moves.len() {
                    // Some clip wasn't where the change expected it.
                    return Err(ApplyError::BeforeMismatch);
                }
                t.resort();
                let index = self.index_mut();
                for &(id, start) in moves.iter() {
                    if let Some(e) = index.get_mut(&id) {
                        e.start = start.checked_add(*delta)?;
                    }
                }
            }
            Change::InsertTrack { seq, slot, track } => {
                for clip in track.clips() {
                    if self.index_entry(clip.id).is_some() {
                        return Err(ApplyError::ClipExists(clip.id));
                    }
                }
                let s = self.seq_mut(*seq)?;
                if s.find_track(track.id).is_some() {
                    return Err(ApplyError::TrackExists(track.id));
                }
                let lane = s.lane_mut(slot.lane);
                if slot.index > lane.len() {
                    return Err(ApplyError::SlotOutOfRange);
                }
                lane.insert(slot.index, Arc::clone(track));
                for clip in track.clips() {
                    self.index_insert(*seq, track.id, clip);
                }
            }
            Change::RemoveTrack { seq, slot, track } => {
                let s = self.seq_mut(*seq)?;
                let lane = s.lane_mut(slot.lane);
                let current = lane.get(slot.index).ok_or(ApplyError::SlotOutOfRange)?;
                if !same(current, track) {
                    return Err(ApplyError::BeforeMismatch);
                }
                lane.remove(slot.index);
                let index = self.index_mut();
                for clip in track.clips() {
                    index.remove(&clip.id);
                }
            }
            Change::SetTrackHeader {
                seq,
                track,
                before,
                after,
            } => {
                let t = self.track_mut(*seq, *track)?;
                if t.header != *before {
                    return Err(ApplyError::BeforeMismatch);
                }
                t.header = after.clone();
            }
            Change::SetTrackAudio {
                seq,
                track,
                before,
                after,
            } => {
                let t = self.track_mut(*seq, *track)?;
                if t.mix != *before {
                    return Err(ApplyError::BeforeMismatch);
                }
                t.mix = *after;
            }
            Change::SetSeqFlags { seq, before, after } => {
                let s = self.seq_mut(*seq)?;
                if s.flags != *before {
                    return Err(ApplyError::BeforeMismatch);
                }
                s.flags = *after;
            }
            Change::SetMaster { seq, before, after } => {
                let s = self.seq_mut(*seq)?;
                if s.master != *before {
                    return Err(ApplyError::BeforeMismatch);
                }
                s.master = *after;
            }
            Change::SetSeqMarkers { seq, before, after } => {
                let s = self.seq_mut(*seq)?;
                if s.markers != *before {
                    return Err(ApplyError::BeforeMismatch);
                }
                s.markers = after.clone();
            }
            Change::SetSettings { before, after } => {
                if self.settings() != before {
                    return Err(ApplyError::BeforeMismatch);
                }
                *self.settings_mut() = *after;
            }
            Change::InsertMedia { item } => {
                if !self.library_mut().insert(Arc::clone(item)) {
                    return Err(ApplyError::MediaExists(item.id));
                }
            }
            Change::RemoveMedia { item } => {
                let current = self
                    .library()
                    .get(item.id)
                    .ok_or(ApplyError::MediaMissing(item.id))?;
                if !same(current, item) {
                    return Err(ApplyError::BeforeMismatch);
                }
                self.library_mut().remove(item.id);
            }
            Change::ReplaceMedia { before, after } => {
                if before.id != after.id {
                    return Err(ApplyError::IdMismatch);
                }
                let current = self
                    .library()
                    .get(before.id)
                    .ok_or(ApplyError::MediaMissing(before.id))?;
                if !same(current, before) {
                    return Err(ApplyError::BeforeMismatch);
                }
                self.library_mut().replace(Arc::clone(after));
            }
        }
        Ok(())
    }

    fn index_insert(&mut self, seq: SeqId, track: TrackId, clip: &Clip) {
        self.index_mut().insert(
            clip.id,
            IndexEntry {
                seq,
                track,
                start: clip.start(),
            },
        );
    }

    fn seq_mut(&mut self, id: SeqId) -> Result<&mut Sequence, ApplyError> {
        self.sequence_mut(id).ok_or(ApplyError::SequenceMissing(id))
    }

    fn track_mut(&mut self, seq: SeqId, track: TrackId) -> Result<&mut Track, ApplyError> {
        self.seq_mut(seq)?
            .track_mut(track)
            .ok_or(ApplyError::TrackMissing(track))
    }

    /// The track holding `clip`, checked against the index.
    fn located_track_mut(
        &mut self,
        seq: SeqId,
        track: TrackId,
        clip: ClipId,
    ) -> Result<&mut Track, ApplyError> {
        match self.index_entry(clip) {
            Some(e) if e.seq == seq && e.track == track => self.track_mut(seq, track),
            Some(_) => Err(ApplyError::BeforeMismatch),
            None => Err(ApplyError::ClipMissing(clip)),
        }
    }
}

fn same<T: PartialEq>(a: &Arc<T>, b: &Arc<T>) -> bool {
    Arc::ptr_eq(a, b) || **a == **b
}
