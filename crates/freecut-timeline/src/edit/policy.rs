//! Policies shared by the edit operations: the magnetic main track (TL-005),
//! collision placement and track creation (TL-006).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use super::tx::Tx;
use super::{Change, EditError, Notice};
use crate::ids::{ClipId, SeqId, TrackId};
use crate::model::{Clip, InvariantViolation, Lane, Sequence, Track, TrackKind, TrackSlot};
use crate::time::{Tick, TimeRange};

pub(crate) fn require_unlocked(track: &Track, slot: TrackSlot) -> Result<(), EditError> {
    match (track.is_locked(), slot.is_main()) {
        (false, _) => Ok(()),
        (true, true) => Err(EditError::MainTrackLocked),
        (true, false) => Err(EditError::TrackLocked { track: track.id }),
    }
}

/// Where an insertion at `t` lands on a magnetic track: the cut nearest the
/// pointer. Over a clip, the first half means "before" and the second half
/// (ties included) "after"; past the end means "append" (TL-005, TL-012).
pub(crate) fn cut_index(clips: &[(ClipId, Tick, Tick)], t: Tick) -> usize {
    for (i, &(_, start, duration)) in clips.iter().enumerate() {
        let end = Tick::from_raw(start.raw().saturating_add(duration.raw()));
        if t < end {
            let offset = i128::from(t.raw()) - i128::from(start.raw());
            return if 2 * offset >= i128::from(duration.raw()) {
                i + 1
            } else {
                i
            };
        }
    }
    clips.len()
}

/// An item in the desired order of a magnetic main track.
#[derive(Debug)]
pub(crate) enum MainItem {
    /// A clip already on the main track.
    Existing(ClipId),
    /// A clip to insert; its start is recomputed. Boxed: ripple layouts
    /// hold thousands of `Existing` items.
    New(Box<Clip>),
}

/// Lays the main track out contiguously from 0 in `order`. Every clip
/// currently on the main track must appear in `order` as `Existing`.
pub(crate) fn layout_main(
    tx: &mut Tx<'_>,
    seq: SeqId,
    order: Vec<MainItem>,
) -> Result<(), EditError> {
    let main = main_track(tx.seq(seq)?)?;
    let by_id: HashMap<ClipId, &Arc<Clip>> = main.clips().iter().map(|c| (c.id, c)).collect();
    let mut pos = Tick::ZERO;
    let mut runs: Vec<(Tick, Vec<(ClipId, Tick)>)> = Vec::new();
    let mut inserts = Vec::new();
    for item in order {
        match item {
            MainItem::Existing(id) => {
                let clip = by_id.get(&id).ok_or(EditError::StaleClip(id))?;
                // Checks that the clip still fits in the tick range there
                // (what `moved_to` checks, without cloning the clip).
                clip.range.with_start(pos)?;
                let delta = pos.checked_sub(clip.start())?;
                if delta != Tick::ZERO {
                    match runs.last_mut() {
                        Some((d, moves)) if *d == delta => moves.push((id, clip.start())),
                        _ => runs.push((delta, vec![(id, clip.start())])),
                    }
                }
                pos = pos.checked_add(clip.duration())?;
            }
            MainItem::New(clip) => {
                let placed = clip.moved_to(pos)?;
                pos = pos.checked_add(placed.duration())?;
                inserts.push(placed);
            }
        }
    }
    for (delta, moves) in runs {
        tx.push(Change::ShiftClips {
            seq,
            track: main.id,
            moves: moves.into(),
            delta,
        })?;
    }
    for clip in inserts {
        tx.push(Change::InsertClip {
            seq,
            track: main.id,
            clip: Arc::new(clip),
        })?;
    }
    Ok(())
}

pub(crate) fn main_track(seq: &Sequence) -> Result<Arc<Track>, EditError> {
    seq.main_track()
        .cloned()
        .ok_or_else(|| EditError::Invariant(vec![InvariantViolation::MainTrack { seq: seq.id }]))
}

/// `(id, start, duration)` of the main track's clips, in order.
pub(crate) fn main_layout(seq: &Sequence) -> Result<Vec<(ClipId, Tick, Tick)>, EditError> {
    Ok(main_track(seq)?
        .clips()
        .iter()
        .map(|c| (c.id, c.start(), c.duration()))
        .collect())
}

/// The slot a new track of `kind` takes, as close to `desired` (an index in
/// the kind's lane) as strict layering allows. `None` means the top of the
/// kind's band, or the bottom of the audio tracks.
pub(crate) fn slot_for_new_track(
    seq: &Sequence,
    kind: TrackKind,
    desired: Option<usize>,
) -> TrackSlot {
    match kind.band() {
        None => {
            let len = seq.audio_tracks().len();
            TrackSlot::audio(desired.unwrap_or(len).min(len))
        }
        Some(band) => {
            let tracks = seq.visual_tracks();
            let below = tracks
                .iter()
                .filter(|t| t.kind.band().is_some_and(|b| b < band))
                .count();
            let within = tracks
                .iter()
                .filter(|t| t.kind.band().is_some_and(|b| b <= band))
                .count();
            // Index 0 is the main track; nothing goes below it.
            let lo = below.max(1);
            let hi = within.max(lo);
            TrackSlot::visual(desired.unwrap_or(hi).clamp(lo, hi))
        }
    }
}

/// Adds an empty track of `kind` at `slot`.
pub(crate) fn add_track_at(
    tx: &mut Tx<'_>,
    seq: SeqId,
    kind: TrackKind,
    slot: TrackSlot,
) -> Result<TrackId, EditError> {
    let max = tx.ctx.limits.max_tracks;
    if tx.seq(seq)?.track_count() >= max {
        return Err(EditError::TrackLimit { max });
    }
    let id = tx.ids.track()?;
    tx.push(Change::InsertTrack {
        seq,
        slot,
        track: Arc::new(Track::new(id, kind)),
    })?;
    Ok(id)
}

/// The slot "directly above" `near` for a new track of `kind`: the next
/// visual position up, or the next audio position down (audio grows away
/// from the main track).
pub(crate) fn slot_next_to(seq: &Sequence, kind: TrackKind, near: TrackSlot) -> TrackSlot {
    let same_lane = (near.lane == Lane::Audio) == kind.is_audio();
    let desired = same_lane.then_some(near.index + 1);
    slot_for_new_track(seq, kind, desired)
}

/// True when none of `ranges` overlaps a clip on `track` outside `ignore`.
pub(crate) fn fits(track: &Track, ranges: &[TimeRange], ignore: &BTreeSet<ClipId>) -> bool {
    ranges
        .iter()
        .all(|r| track.clips_in_range(*r).all(|c| ignore.contains(&c.id)))
}

/// Places `clips` on `target`. If the target is locked or occupied, they go
/// onto a new track directly above it instead (TL-006, TL-009). Returns the
/// track used.
pub(crate) fn place_clips(
    tx: &mut Tx<'_>,
    seq: SeqId,
    target: TrackId,
    clips: Vec<Clip>,
) -> Result<TrackId, EditError> {
    let (slot, track) = tx
        .seq(seq)?
        .find_track(target)
        .map(|(s, t)| (s, Arc::clone(t)))
        .ok_or(EditError::StaleTrack(target))?;
    let ranges: Vec<TimeRange> = clips.iter().map(|c| c.range).collect();
    let track_id = if !track.is_locked() && fits(&track, &ranges, &BTreeSet::new()) {
        target
    } else {
        let slot = slot_next_to(tx.seq(seq)?, track.kind, slot);
        let id = add_track_at(tx, seq, track.kind, slot)?;
        tx.notice(Notice::CreatedTrack(id));
        id
    };
    for clip in clips {
        tx.push(Change::InsertClip {
            seq,
            track: track_id,
            clip: Arc::new(clip),
        })?;
    }
    Ok(track_id)
}

/// Shifts clips on one track, each by its own delta, grouping equal shifts.
/// Checks every new position fits in the tick range first, so an overflow
/// is reported as [`EditError::Time`] rather than as a planner bug.
pub(crate) fn shift(
    tx: &mut Tx<'_>,
    seq: SeqId,
    track: TrackId,
    moves: Vec<(Arc<Clip>, Tick)>,
) -> Result<(), EditError> {
    let mut runs: BTreeMap<Tick, Vec<(ClipId, Tick)>> = BTreeMap::new();
    for (clip, delta) in moves {
        if delta == Tick::ZERO {
            continue;
        }
        clip.moved_to(clip.start().checked_add(delta)?)?;
        runs.entry(delta).or_default().push((clip.id, clip.start()));
    }
    for (delta, moves) in runs {
        tx.push(Change::ShiftClips {
            seq,
            track,
            moves: moves.into(),
            delta,
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::num::NonZeroU64;

    fn c(n: u64, start: i64, dur: i64) -> (ClipId, Tick, Tick) {
        (
            ClipId::from_raw(NonZeroU64::new(n).unwrap()),
            Tick::from_raw(start),
            Tick::from_raw(dur),
        )
    }

    #[test]
    fn cut_index_uses_the_midpoint_with_ties_after() {
        let clips = [c(1, 0, 10), c(2, 10, 10)];
        let at = |t| cut_index(&clips, Tick::from_raw(t));
        assert_eq!(at(-5), 0);
        assert_eq!(at(0), 0);
        assert_eq!(at(4), 0);
        assert_eq!(at(5), 1, "exact midpoint goes after");
        assert_eq!(at(14), 1);
        assert_eq!(at(15), 2);
        assert_eq!(at(25), 2, "past the end appends");
        assert_eq!(cut_index(&[], Tick::from_raw(100)), 0);
    }
}
