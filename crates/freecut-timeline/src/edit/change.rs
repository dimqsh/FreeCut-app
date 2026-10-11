//! Primitive changes and change sets (TL-043).
//!
//! An edit is recorded as a list of primitive before/after changes. Undo is
//! the mechanical inverse; no operation has hand-written undo code.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::ids::{ClipId, MediaId, SeqId, TrackId};
use crate::model::{
    Clip, Document, Markers, MasterAudio, MediaItem, ProjectSettings, Scope, SequenceFlags, Track,
    TrackAudio, TrackHeader, TrackSlot,
};
use crate::time::Tick;

use super::ApplyError;

/// One primitive, invertible change. Every variant carries enough of the
/// state it expects to find that applying it to the wrong document fails
/// with [`ApplyError`] instead of silently producing a different document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    InsertClip {
        seq: SeqId,
        track: TrackId,
        clip: Arc<Clip>,
    },
    /// Carries a full snapshot so the inverse can restore it.
    RemoveClip {
        seq: SeqId,
        track: TrackId,
        clip: Arc<Clip>,
    },
    /// Same id, same track.
    ReplaceClip {
        seq: SeqId,
        track: TrackId,
        before: Arc<Clip>,
        after: Arc<Clip>,
    },
    /// Moves clips on one track by `delta` (a compact ripple). `moves` holds
    /// each clip's id and its start before the shift.
    ShiftClips {
        seq: SeqId,
        track: TrackId,
        moves: Arc<[(ClipId, Tick)]>,
        delta: Tick,
    },
    /// May carry clips.
    InsertTrack {
        seq: SeqId,
        slot: TrackSlot,
        track: Arc<Track>,
    },
    /// Carries a full snapshot, clips included.
    RemoveTrack {
        seq: SeqId,
        slot: TrackSlot,
        track: Arc<Track>,
    },
    SetTrackHeader {
        seq: SeqId,
        track: TrackId,
        before: TrackHeader,
        after: TrackHeader,
    },
    /// The track stage of a track (AU-013).
    SetTrackAudio {
        seq: SeqId,
        track: TrackId,
        before: TrackAudio,
        after: TrackAudio,
    },
    SetSeqFlags {
        seq: SeqId,
        before: SequenceFlags,
        after: SequenceFlags,
    },
    /// The master stage of a sequence (AU-010).
    SetMaster {
        seq: SeqId,
        before: MasterAudio,
        after: MasterAudio,
    },
    /// The timeline markers of a sequence (TL-037).
    SetSeqMarkers {
        seq: SeqId,
        before: Markers,
        after: Markers,
    },
    SetSettings {
        before: ProjectSettings,
        after: ProjectSettings,
    },
    InsertMedia {
        item: Arc<MediaItem>,
    },
    /// Carries a full snapshot so the inverse can restore it.
    RemoveMedia {
        item: Arc<MediaItem>,
    },
    /// Same id.
    ReplaceMedia {
        before: Arc<MediaItem>,
        after: Arc<MediaItem>,
    },
}

impl Change {
    /// The change that undoes this one.
    pub fn inverse(&self) -> Change {
        match self.clone() {
            Change::InsertClip { seq, track, clip } => Change::RemoveClip { seq, track, clip },
            Change::RemoveClip { seq, track, clip } => Change::InsertClip { seq, track, clip },
            Change::ReplaceClip {
                seq,
                track,
                before,
                after,
            } => Change::ReplaceClip {
                seq,
                track,
                before: after,
                after: before,
            },
            Change::ShiftClips {
                seq,
                track,
                moves,
                delta,
            } => Change::ShiftClips {
                seq,
                track,
                // The forward shift was applied successfully, so every
                // `start + delta` is representable and `-delta` too (planners
                // never emit i64::MIN); saturation never triggers.
                moves: moves
                    .iter()
                    .map(|&(id, start)| {
                        (id, Tick::from_raw(start.raw().saturating_add(delta.raw())))
                    })
                    .collect(),
                delta: Tick::from_raw(delta.raw().saturating_neg()),
            },
            Change::InsertTrack { seq, slot, track } => Change::RemoveTrack { seq, slot, track },
            Change::RemoveTrack { seq, slot, track } => Change::InsertTrack { seq, slot, track },
            Change::SetTrackHeader {
                seq,
                track,
                before,
                after,
            } => Change::SetTrackHeader {
                seq,
                track,
                before: after,
                after: before,
            },
            Change::SetTrackAudio {
                seq,
                track,
                before,
                after,
            } => Change::SetTrackAudio {
                seq,
                track,
                before: after,
                after: before,
            },
            Change::SetSeqFlags { seq, before, after } => Change::SetSeqFlags {
                seq,
                before: after,
                after: before,
            },
            Change::SetMaster { seq, before, after } => Change::SetMaster {
                seq,
                before: after,
                after: before,
            },
            Change::SetSeqMarkers { seq, before, after } => Change::SetSeqMarkers {
                seq,
                before: after,
                after: before,
            },
            Change::SetSettings { before, after } => Change::SetSettings {
                before: after,
                after: before,
            },
            Change::InsertMedia { item } => Change::RemoveMedia { item },
            Change::RemoveMedia { item } => Change::InsertMedia { item },
            Change::ReplaceMedia { before, after } => Change::ReplaceMedia {
                before: after,
                after: before,
            },
        }
    }

    /// True when applying the change can't alter anything.
    fn is_noop(&self) -> bool {
        match self {
            Change::ReplaceClip { before, after, .. } => {
                Arc::ptr_eq(before, after) || before == after
            }
            Change::ShiftClips { moves, delta, .. } => moves.is_empty() || *delta == Tick::ZERO,
            Change::SetTrackHeader { before, after, .. } => before == after,
            Change::SetTrackAudio { before, after, .. } => before == after,
            Change::SetSeqFlags { before, after, .. } => before == after,
            Change::SetMaster { before, after, .. } => before == after,
            Change::SetSeqMarkers { before, after, .. } => before == after,
            Change::SetSettings { before, after } => before == after,
            Change::ReplaceMedia { before, after } => Arc::ptr_eq(before, after) || before == after,
            _ => false,
        }
    }

    /// The entities this change reads or writes, for folding barriers.
    fn keys(&self) -> Vec<Key> {
        match self {
            Change::InsertClip { clip, .. } | Change::RemoveClip { clip, .. } => {
                vec![Key::Clip(clip.id)]
            }
            Change::ReplaceClip { before, .. } => vec![Key::Clip(before.id)],
            Change::ShiftClips { moves, .. } => {
                moves.iter().map(|(id, _)| Key::Clip(*id)).collect()
            }
            Change::InsertTrack { track, .. } | Change::RemoveTrack { track, .. } => {
                let mut keys = vec![Key::Track(track.id)];
                keys.extend(track.clips().iter().map(|c| Key::Clip(c.id)));
                keys
            }
            Change::SetTrackHeader { track, .. } | Change::SetTrackAudio { track, .. } => {
                vec![Key::Track(*track)]
            }
            Change::SetSeqFlags { seq, .. } => vec![Key::Seq(*seq)],
            Change::SetMaster { seq, .. } => vec![Key::Master(*seq)],
            Change::SetSeqMarkers { seq, .. } => vec![Key::Markers(*seq)],
            Change::SetSettings { .. } => vec![Key::Settings],
            Change::InsertMedia { item } | Change::RemoveMedia { item } => {
                vec![Key::Media(item.id)]
            }
            Change::ReplaceMedia { before, .. } => vec![Key::Media(before.id)],
        }
    }

    /// True when this change places, changes or removes a clip that plays
    /// media `id`.
    fn references_media(&self, id: MediaId) -> bool {
        let uses = |c: &Clip| c.content.asset() == Some(id);
        match self {
            Change::InsertClip { clip, .. } | Change::RemoveClip { clip, .. } => uses(clip),
            Change::ReplaceClip { before, after, .. } => uses(before) || uses(after),
            Change::InsertTrack { track, .. } | Change::RemoveTrack { track, .. } => {
                track.clips().iter().any(|c| uses(c))
            }
            Change::ReplaceMedia { before, .. } => before.id == id,
            Change::InsertMedia { item } | Change::RemoveMedia { item } => item.id == id,
            _ => false,
        }
    }

    /// True when this change reads or writes track `id` or its clips.
    fn references_track(&self, id: TrackId) -> bool {
        match self {
            Change::InsertClip { track, .. }
            | Change::RemoveClip { track, .. }
            | Change::ReplaceClip { track, .. }
            | Change::ShiftClips { track, .. }
            | Change::SetTrackHeader { track, .. }
            | Change::SetTrackAudio { track, .. } => *track == id,
            Change::InsertTrack { track, .. } | Change::RemoveTrack { track, .. } => track.id == id,
            Change::SetSeqFlags { .. }
            | Change::SetMaster { .. }
            | Change::SetSeqMarkers { .. }
            | Change::SetSettings { .. }
            | Change::InsertMedia { .. }
            | Change::RemoveMedia { .. }
            | Change::ReplaceMedia { .. } => false,
        }
    }

    fn is_track_structure(&self, in_seq: SeqId) -> bool {
        matches!(self, Change::InsertTrack { seq, .. } | Change::RemoveTrack { seq, .. } if *seq == in_seq)
    }

    /// Approximate heap and inline size, for the history's memory budget.
    /// Shared `Arc`s are counted at every use, so this over-counts.
    fn heap_bytes(&self) -> usize {
        let clip = |c: &Clip| {
            core::mem::size_of::<Clip>()
                + c.name.as_ref().map_or(0, String::len)
                + c.markers.heap_bytes()
                + c.audio
                    .active()
                    .map_or(0, |a| core::mem::size_of_val(a.gain.keys()))
                + c.content.text().map_or(0, |t| {
                    core::mem::size_of::<crate::model::TextClipData>()
                        + t.content.len()
                        + t.style.font.family.len()
                        + t.style
                            .fill
                            .stops()
                            .map_or(0, |st| core::mem::size_of_val(st.as_slice()))
                })
        };
        core::mem::size_of::<Change>()
            + match self {
                Change::InsertClip { clip: c, .. } | Change::RemoveClip { clip: c, .. } => clip(c),
                Change::ReplaceClip { before, after, .. } => clip(before) + clip(after),
                Change::ShiftClips { moves, .. } => {
                    moves.len() * core::mem::size_of::<(ClipId, Tick)>()
                }
                Change::InsertTrack { track, .. } | Change::RemoveTrack { track, .. } => {
                    core::mem::size_of::<Track>()
                        + track.header.name.as_ref().map_or(0, String::len)
                        + track.clips().iter().map(|c| clip(c)).sum::<usize>()
                }
                Change::SetTrackHeader { before, after, .. } => {
                    before.name.as_ref().map_or(0, String::len)
                        + after.name.as_ref().map_or(0, String::len)
                }
                Change::SetTrackAudio { .. }
                | Change::SetSeqFlags { .. }
                | Change::SetMaster { .. }
                | Change::SetSettings { .. } => 0,
                Change::SetSeqMarkers { before, after, .. } => {
                    before.heap_bytes() + after.heap_bytes()
                }
                Change::InsertMedia { item } | Change::RemoveMedia { item } => media_bytes(item),
                Change::ReplaceMedia { before, after } => media_bytes(before) + media_bytes(after),
            }
    }

    /// Records which sequences and tracks this change touched, for scoped
    /// validation.
    pub(crate) fn touch(&self, scope: &mut Scope) {
        let related = |c: &Clip| c.group.is_some() || c.link.is_some();
        let new_markers = |scope: &mut Scope, before: Option<&Markers>, after: &Markers| {
            for m in after {
                if before.is_none_or(|b| b.get(m.id).is_none()) {
                    scope.new_id(m.id.get());
                }
            }
        };
        match self {
            Change::InsertClip { seq, track, clip } => {
                scope.track(*seq, *track);
                scope.new_id(clip.id.get());
                new_markers(scope, None, &clip.markers);
                scope.note_relations(related(clip));
                scope.note_attachments(clip.attach.is_some());
            }
            Change::RemoveClip { seq, track, clip } => {
                scope.track(*seq, *track);
                scope.note_relations(related(clip));
                // The clip may have been an owner.
                scope.note_attachments(true);
            }
            Change::ReplaceClip {
                seq,
                track,
                before,
                after,
            } => {
                scope.track(*seq, *track);
                new_markers(scope, Some(&before.markers), &after.markers);
                scope.note_relations(related(before) || related(after));
                scope.note_attachments(before.attach.is_some() || after.attach.is_some());
            }
            Change::ShiftClips { seq, track, .. }
            | Change::SetTrackHeader { seq, track, .. }
            | Change::SetTrackAudio { seq, track, .. } => {
                scope.track(*seq, *track);
            }
            Change::InsertTrack { seq, track, .. } => {
                scope.structure(*seq);
                scope.track(*seq, track.id);
                scope.new_id(track.id.get());
                for clip in track.clips() {
                    scope.new_id(clip.id.get());
                    new_markers(scope, None, &clip.markers);
                    scope.note_relations(related(clip));
                    scope.note_attachments(clip.attach.is_some());
                }
            }
            Change::RemoveTrack { seq, track, .. } => {
                scope.structure(*seq);
                scope.note_relations(track.clips().iter().any(|c| related(c)));
                scope.note_attachments(!track.is_empty());
            }
            Change::SetSeqFlags { seq, .. } => {
                scope.structure(*seq);
                scope.note_attachments(true);
            }
            Change::SetMaster { seq, .. } => {
                scope.structure(*seq);
            }
            Change::SetSeqMarkers { seq, before, after } => {
                scope.structure(*seq);
                new_markers(scope, Some(before), after);
            }
            Change::SetSettings { .. } => scope.settings = true,
            Change::InsertMedia { item } => {
                scope.library = true;
                scope.new_id(item.id.get());
            }
            Change::RemoveMedia { .. } | Change::ReplaceMedia { .. } => scope.library = true,
        }
    }
}

fn media_bytes(item: &MediaItem) -> usize {
    core::mem::size_of::<MediaItem>()
        + item.path.absolute.len()
        + item.path.relative.as_ref().map_or(0, String::len)
        + item.info.container.len()
        + (item.info.video.len() * core::mem::size_of::<crate::model::VideoStreamInfo>())
        + (item.info.audio.len() * core::mem::size_of::<crate::model::AudioStreamInfo>())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Settings,
    Media(MediaId),
    Seq(SeqId),
    Master(SeqId),
    Markers(SeqId),
    Track(TrackId),
    Clip(ClipId),
}

/// The primitive changes of one edit, in application order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeSet(Vec<Change>);

impl ChangeSet {
    pub(crate) fn new(changes: Vec<Change>) -> ChangeSet {
        ChangeSet(changes)
    }

    pub fn changes(&self) -> &[Change] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// The changes that undo this set, in application order.
    pub fn inverse(&self) -> impl Iterator<Item = Change> + '_ {
        self.0.iter().rev().map(Change::inverse)
    }

    pub(crate) fn heap_bytes(&self) -> usize {
        self.0.iter().map(Change::heap_bytes).sum()
    }

    pub(crate) fn extend(&mut self, other: ChangeSet) {
        self.0.extend(other.0);
    }

    pub(crate) fn scope(&self) -> Scope {
        scope_of(&self.0)
    }

    /// Folds redundant pairs so merged gestures stay small:
    /// - `Replace(x: a→b)` then `Replace(x: b→c)` becomes `Replace(x: a→c)`,
    ///   and likewise for track headers, track stages, sequence flags,
    ///   master stages and timeline markers;
    /// - `Shift(t, ids, d1)` then `Shift(t, ids, d2)` becomes `Shift(d1+d2)`;
    /// - `Remove(x)` then `Insert(x)` on the same track becomes a replace;
    /// - `Insert(x)` … `Remove(x)` cancels, for clips and for tracks;
    /// - no-op changes are dropped.
    ///
    /// Folding never crosses a change that touches the same entity.
    pub(crate) fn compact(&mut self) {
        let mut out: Vec<Change> = Vec::with_capacity(self.0.len());
        for change in self.0.drain(..) {
            if change.is_noop() {
                continue;
            }
            if let Some(c) = fold_into(&mut out, change) {
                out.push(c);
            }
        }
        out.retain(|c| !c.is_noop());
        self.0 = out;
    }
}

/// Tries to fold `next` into the latest earlier change touching the same keys.
/// Returns the change back if it must be appended.
fn fold_into(out: &mut Vec<Change>, next: Change) -> Option<Change> {
    let next_keys: BTreeSet<Key> = next.keys().into_iter().collect();
    // The most recent change touching any of next's keys is the only fold
    // candidate; anything earlier is behind a barrier.
    let Some(i) = out
        .iter()
        .rposition(|c| c.keys().iter().any(|k| next_keys.contains(k)))
    else {
        return Some(next);
    };
    let merged = match (&out[i], &next) {
        (
            Change::ReplaceClip {
                seq, track, before, ..
            },
            Change::ReplaceClip {
                seq: s2,
                track: t2,
                after,
                ..
            },
        ) if seq == s2 && track == t2 => Some(Fold::Replace(Change::ReplaceClip {
            seq: *seq,
            track: *track,
            before: Arc::clone(before),
            after: Arc::clone(after),
        })),
        (
            Change::ShiftClips {
                seq,
                track,
                moves,
                delta,
            },
            Change::ShiftClips {
                seq: s2,
                track: t2,
                moves: moves2,
                delta: d2,
            },
        ) if seq == s2 && track == t2 && shifted_by(moves, *delta, moves2) => {
            delta.checked_add(*d2).ok().map(|sum| {
                Fold::Replace(Change::ShiftClips {
                    seq: *seq,
                    track: *track,
                    moves: Arc::clone(moves),
                    delta: sum,
                })
            })
        }
        (
            Change::InsertClip { seq, track, clip },
            Change::RemoveClip {
                seq: s2,
                track: t2,
                clip: c2,
            },
        ) if seq == s2 && track == t2 && clip.id == c2.id => Some(Fold::Cancel),
        (
            Change::RemoveClip { seq, track, clip },
            Change::InsertClip {
                seq: s2,
                track: t2,
                clip: c2,
            },
        ) if seq == s2 && track == t2 && clip.id == c2.id => {
            Some(Fold::Replace(Change::ReplaceClip {
                seq: *seq,
                track: *track,
                before: Arc::clone(clip),
                after: Arc::clone(c2),
            }))
        }
        (
            Change::SetTrackHeader {
                seq, track, before, ..
            },
            Change::SetTrackHeader {
                seq: s2,
                track: t2,
                after,
                ..
            },
        ) if seq == s2 && track == t2 => Some(Fold::Replace(Change::SetTrackHeader {
            seq: *seq,
            track: *track,
            before: before.clone(),
            after: after.clone(),
        })),
        (
            Change::SetTrackAudio {
                seq, track, before, ..
            },
            Change::SetTrackAudio {
                seq: s2,
                track: t2,
                after,
                ..
            },
        ) if seq == s2 && track == t2 => Some(Fold::Replace(Change::SetTrackAudio {
            seq: *seq,
            track: *track,
            before: *before,
            after: *after,
        })),
        (Change::SetSeqFlags { seq, before, .. }, Change::SetSeqFlags { seq: s2, after, .. })
            if seq == s2 =>
        {
            Some(Fold::Replace(Change::SetSeqFlags {
                seq: *seq,
                before: *before,
                after: *after,
            }))
        }
        (Change::SetMaster { seq, before, .. }, Change::SetMaster { seq: s2, after, .. })
            if seq == s2 =>
        {
            Some(Fold::Replace(Change::SetMaster {
                seq: *seq,
                before: *before,
                after: *after,
            }))
        }
        (
            Change::SetSeqMarkers { seq, before, .. },
            Change::SetSeqMarkers { seq: s2, after, .. },
        ) if seq == s2 => Some(Fold::Replace(Change::SetSeqMarkers {
            seq: *seq,
            before: before.clone(),
            after: after.clone(),
        })),
        (Change::SetSettings { before, .. }, Change::SetSettings { after, .. }) => {
            Some(Fold::Replace(Change::SetSettings {
                before: *before,
                after: *after,
            }))
        }
        (Change::ReplaceMedia { before, .. }, Change::ReplaceMedia { after, .. }) => {
            Some(Fold::Replace(Change::ReplaceMedia {
                before: Arc::clone(before),
                after: Arc::clone(after),
            }))
        }
        (Change::InsertMedia { .. }, Change::ReplaceMedia { after, .. }) => {
            Some(Fold::Replace(Change::InsertMedia {
                item: Arc::clone(after),
            }))
        }
        (Change::ReplaceMedia { before, .. }, Change::RemoveMedia { .. }) => {
            Some(Fold::Replace(Change::RemoveMedia {
                item: Arc::clone(before),
            }))
        }
        // A media item imported and removed again, with nothing in between
        // using it.
        (Change::InsertMedia { item }, Change::RemoveMedia { item: i2 })
            if item.id == i2.id && !out[i + 1..].iter().any(|c| c.references_media(item.id)) =>
        {
            Some(Fold::Cancel)
        }
        // A clip inserted and then changed is inserted as changed.
        (
            Change::InsertClip { seq, track, .. },
            Change::ReplaceClip {
                seq: s2,
                track: t2,
                after,
                ..
            },
        ) if seq == s2 && track == t2 => Some(Fold::Replace(Change::InsertClip {
            seq: *seq,
            track: *track,
            clip: Arc::clone(after),
        })),
        // A clip changed and then removed is removed as it was.
        (
            Change::ReplaceClip {
                seq, track, before, ..
            },
            Change::RemoveClip {
                seq: s2, track: t2, ..
            },
        ) if seq == s2 && track == t2 => Some(Fold::Replace(Change::RemoveClip {
            seq: *seq,
            track: *track,
            clip: Arc::clone(before),
        })),
        // A track added and removed again, unchanged, with nothing in between
        // referring to it, and no other track added or removed in between
        // (that would shift slot indices).
        (
            Change::InsertTrack { seq, slot, track },
            Change::RemoveTrack {
                seq: s2,
                slot: slot2,
                track: t2,
            },
        ) if seq == s2
            && slot == slot2
            && track == t2
            && !out[i + 1..]
                .iter()
                .any(|c| c.is_track_structure(*seq) || c.references_track(track.id)) =>
        {
            Some(Fold::Cancel)
        }
        _ => None,
    };
    match merged {
        Some(Fold::Replace(c)) => {
            out[i] = c;
            None
        }
        Some(Fold::Cancel) => {
            out.remove(i);
            None
        }
        None => Some(next),
    }
}

/// True when `later` moves exactly the clips of `earlier`, from where
/// `earlier` left them.
fn shifted_by(earlier: &[(ClipId, Tick)], delta: Tick, later: &[(ClipId, Tick)]) -> bool {
    earlier.len() == later.len()
        && earlier
            .iter()
            .zip(later)
            .all(|(&(a, s), &(b, s2))| a == b && s.checked_add(delta).is_ok_and(|e| e == s2))
}

enum Fold {
    Replace(Change),
    Cancel,
}

/// What `changes` touched, for scoped validation.
pub(crate) fn scope_of(changes: &[Change]) -> Scope {
    let mut scope = Scope::default();
    for change in changes {
        change.touch(&mut scope);
    }
    scope
}

/// Applies `changes` to a clone of `doc`. Atomic: on error the clone is
/// dropped and `doc` is untouched.
pub(crate) fn apply_all<I: IntoIterator<Item = Change>>(
    doc: &Document,
    changes: I,
) -> Result<Document, ApplyError> {
    let mut next = doc.clone();
    for change in changes {
        next.apply(&change)?;
    }
    Ok(next)
}
