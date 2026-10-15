//! Attached items following their owners (TL-029), run by
//! [`Tx::finish`](super::tx::Tx::finish) after every op.
//!
//! Two passes per touched sequence:
//! 1. **follow_owners**: items the op didn't move themselves, attached to a
//!    main-track clip that moved, move by the same amount as their anchor
//!    frame (re-anchoring after a split or a trim that removed it); items of
//!    a deleted owner are deleted with it (except the owner's own detached
//!    sound, TL-030). Locked tracks keep their items
//!    (they show as out of sync), and an item that would collide moves to a
//!    new track directly above.
//! 2. **reattach**: items the op placed, moved or trimmed get the attachment
//!    their new position gives them.
//!
//! Everything goes through [`Tx::push`], so it is part of the op's undo.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::policy;
use super::tx::Tx;
use super::{Change, EditError, Notice};
use crate::ids::{ClipId, SeqId, TrackId};
use crate::model::{Attachment, Clip, Document, Sequence, TrackSlot, attachment_for, follow_kind};
use crate::time::{Tick, TimeRange};

pub(super) fn follow_owners(tx: &mut Tx<'_>) -> Result<(), EditError> {
    let scope = tx.scope();
    let base = tx.base();
    let mut removed = 0u32;
    for &seq_id in &scope.seqs {
        let Some(base_seq) = base.sequence(seq_id) else {
            continue;
        };
        let Some(work_seq) = tx.work().sequence(seq_id) else {
            continue;
        };
        if !base_seq.flags.follow_main && !work_seq.flags.follow_main {
            continue;
        }
        let tracks: BTreeSet<TrackId> = scope
            .tracks
            .iter()
            .filter(|(s, _)| *s == seq_id)
            .map(|(_, t)| *t)
            .collect();
        let mut touched = touched_clips(base, work_seq, &tracks);
        let known: BTreeSet<ClipId> = touched.iter().copied().collect();
        for id in tx.placed_ids() {
            let here = tx.work().find_clip(*id).is_some_and(|l| l.seq == seq_id);
            if here && !known.contains(id) {
                touched.push(*id);
            }
        }
        let main_changed = base_seq
            .main_track()
            .is_some_and(|m| tracks.contains(&m.id));
        if base_seq.flags.follow_main && main_changed {
            removed += follow_step(tx, base_seq, seq_id, &touched)?;
        }
        reattach(tx, seq_id, &touched)?;
    }
    if removed > 0 {
        tx.notice(Notice::AlsoRemovedAttached(removed));
    }
    Ok(())
}

/// Clips of the touched tracks the op placed, moved or trimmed (new, on
/// another track, or another range), plus main-track clips still carrying
/// an attachment.
fn touched_clips(base: &Document, work: &Sequence, tracks: &BTreeSet<TrackId>) -> Vec<ClipId> {
    let mut out = Vec::new();
    for (slot, track) in work.tracks() {
        if !tracks.contains(&track.id) {
            continue;
        }
        for clip in track.clips() {
            let changed = if slot.is_main() {
                clip.attach.is_some()
            } else {
                match base.find_clip(clip.id) {
                    None => true,
                    Some(loc) => {
                        loc.track != track.id
                            || base.clip(clip.id).is_none_or(|b| b.range != clip.range)
                    }
                }
            };
            if changed {
                out.push(clip.id);
            }
        }
    }
    out
}

/// What happens to one attached item.
enum Follow {
    /// Delete it with its owner.
    Delete,
    /// Keep it in place with this attachment.
    Anchor(Option<Attachment>),
    /// Move it to this start, with this attachment.
    Move(Tick, Attachment),
}

/// Pass 1; returns how many items were deleted with their owner.
fn follow_step(
    tx: &mut Tx<'_>,
    base_seq: &Sequence,
    seq_id: SeqId,
    touched: &[ClipId],
) -> Result<u32, EditError> {
    let touched: BTreeSet<ClipId> = touched.iter().copied().collect();
    let base = tx.base();
    let rate = tx.rate();
    let mut new_main: Option<Vec<Arc<Clip>>> = None;
    let mut actions: Vec<(TrackId, Arc<Clip>, Follow)> = Vec::new();
    {
        let work = tx.work();
        let work_seq = tx.seq(seq_id)?;
        for (slot, track) in base_seq.tracks() {
            if slot.is_main() {
                continue;
            }
            for item in track.clips() {
                let Some(attach) = item.attach else { continue };
                if touched.contains(&item.id) {
                    continue;
                }
                let (Some(now), Some(loc)) = (work.clip(item.id), work.find_clip(item.id)) else {
                    continue;
                };
                let Some(now_track) = work_seq.track_at(loc.slot) else {
                    continue;
                };
                let follows = work_seq.flags.follow_main
                    && work_seq.flags.follow_kinds.contains(follow_kind(
                        now,
                        now_track,
                        work.library(),
                    ));
                if !follows {
                    if now.attach.is_some() {
                        actions.push((now_track.id, Arc::clone(now), Follow::Anchor(None)));
                    }
                    continue;
                }
                let Some(before) = base
                    .clip(attach.owner)
                    .and_then(|o| o.sequence_time_of(attach.owner_source_time))
                else {
                    continue;
                };
                let locked = now_track.is_locked();
                // An owner's own detached sound outlives it (TL-030).
                let partner = now.link.is_some()
                    && base.clip(attach.owner).is_some_and(|o| o.link == now.link);
                let action = match work.find_clip(attach.owner) {
                    None if locked || partner => Follow::Anchor(None),
                    None => Follow::Delete,
                    Some(o) if o.seq != seq_id || !o.slot.is_main() => Follow::Anchor(
                        attachment_for(work_seq, loc.slot, now_track, now, work.library()),
                    ),
                    Some(_) => {
                        let owner = work
                            .clip(attach.owner)
                            .ok_or(EditError::StaleClip(attach.owner))?;
                        let new_main = new_main.get_or_insert_with(|| {
                            work_seq
                                .main_track()
                                .map(|m| {
                                    m.clips()
                                        .iter()
                                        .filter(|c| base.find_clip(c.id).is_none())
                                        .cloned()
                                        .collect()
                                })
                                .unwrap_or_default()
                        });
                        let anchor = reanchor(
                            owner,
                            attach.owner_source_time,
                            new_main,
                            rate.frame_ticks(),
                        );
                        let (owner, time) = anchor;
                        let after = owner
                            .sequence_time_of(time)
                            .ok_or(EditError::StaleClip(owner.id))?;
                        let delta = after.checked_sub(before)?;
                        let attach = Attachment {
                            owner: owner.id,
                            owner_source_time: time,
                        };
                        let start = now.start().checked_add(delta)?;
                        if delta == Tick::ZERO || locked || start.is_negative() {
                            Follow::Anchor(Some(attach))
                        } else {
                            Follow::Move(start, attach)
                        }
                    }
                };
                let unchanged = matches!(action, Follow::Anchor(a) if a == now.attach);
                if !unchanged {
                    actions.push((now_track.id, Arc::clone(now), action));
                }
            }
        }
    }

    let mut removed = 0u32;
    let mut movers: BTreeMap<TrackId, Vec<(Arc<Clip>, Clip)>> = BTreeMap::new();
    for (track, clip, action) in actions {
        match action {
            Follow::Delete => {
                tx.push(Change::RemoveClip {
                    seq: seq_id,
                    track,
                    clip,
                })?;
                removed = removed.saturating_add(1);
            }
            Follow::Anchor(attach) => {
                let mut after = (*clip).clone();
                after.attach = attach;
                tx.push(Change::ReplaceClip {
                    seq: seq_id,
                    track,
                    before: clip,
                    after: Arc::new(after),
                })?;
            }
            Follow::Move(start, attach) => {
                let mut after = clip.moved_to(start)?;
                after.attach = Some(attach);
                movers.entry(track).or_default().push((clip, after));
            }
        }
    }
    for (track, list) in movers {
        land(tx, seq_id, track, list)?;
    }
    Ok(removed)
}

/// The owner and anchor frame an item hangs on after the op: the same when
/// the owner still shows the frame; the new main-track piece showing it
/// after a split; else the owner's nearest remaining frame.
fn reanchor<'c>(
    owner: &'c Arc<Clip>,
    time: Tick,
    new_main: &'c [Arc<Clip>],
    frame: Tick,
) -> (&'c Arc<Clip>, Tick) {
    let shows = |c: &Clip| c.content_range().is_some_and(|r| r.contains(time));
    if shows(owner) {
        return (owner, time);
    }
    if let Some(piece) = new_main
        .iter()
        .find(|c| c.content.asset() == owner.content.asset() && shows(c))
    {
        return (piece, time);
    }
    let Some(range) = owner.content_range() else {
        return (owner, time);
    };
    let last = range
        .end()
        .checked_sub(frame)
        .unwrap_or(range.start())
        .max(range.start());
    (owner, time.clamp(range.start(), last))
}

/// Moves followers on one track; any that would collide go to a new track
/// directly above (TL-029).
fn land(
    tx: &mut Tx<'_>,
    seq_id: SeqId,
    track_id: TrackId,
    mut list: Vec<(Arc<Clip>, Clip)>,
) -> Result<(), EditError> {
    list.sort_by_key(|(_, after)| (after.start(), after.id));
    let moving: BTreeSet<ClipId> = list.iter().map(|(b, _)| b.id).collect();
    let (slot, track) = tx
        .seq(seq_id)?
        .find_track(track_id)
        .map(|(s, t)| (s, Arc::clone(t)))
        .ok_or(EditError::StaleTrack(track_id))?;
    let mut accepted: Vec<TimeRange> = Vec::new();
    let mut overflow = Vec::new();
    for (before, after) in list {
        let r = after.range;
        let blocked = track.clips_in_range(r).any(|c| !moving.contains(&c.id))
            || accepted.iter().any(|a| a.overlaps(r));
        if blocked {
            overflow.push((before, after));
        } else {
            accepted.push(r);
            tx.push(Change::ReplaceClip {
                seq: seq_id,
                track: track_id,
                before,
                after: Arc::new(after),
            })?;
        }
    }
    let mut extra: Vec<(TrackSlot, TrackId)> = Vec::new();
    for (before, after) in overflow {
        tx.push(Change::RemoveClip {
            seq: seq_id,
            track: track_id,
            clip: before,
        })?;
        let r = [after.range];
        let free = extra.iter().find(|(_, id)| {
            tx.seq(seq_id)
                .ok()
                .and_then(|s| s.find_track(*id))
                .is_some_and(|(_, t)| policy::fits(t, &r, &BTreeSet::new()))
        });
        let target = match free {
            Some((_, id)) => *id,
            None => {
                let near = extra.last().map_or(slot, |(s, _)| *s);
                let new_slot = policy::slot_next_to(tx.seq(seq_id)?, track.kind, near);
                let id = policy::add_track_at(tx, seq_id, track.kind, new_slot)?;
                tx.notice(Notice::CreatedTrack(id));
                // Slots shift as tracks are added; keep the new one's.
                let s = tx
                    .seq(seq_id)?
                    .find_track(id)
                    .map(|(s, _)| s)
                    .ok_or(EditError::StaleTrack(id))?;
                extra.push((s, id));
                id
            }
        };
        tx.push(Change::InsertClip {
            seq: seq_id,
            track: target,
            clip: Arc::new(after),
        })?;
    }
    Ok(())
}

/// Pass 2: the clips the op placed, moved or trimmed take the attachment
/// their position gives them.
fn reattach(tx: &mut Tx<'_>, seq_id: SeqId, touched: &[ClipId]) -> Result<(), EditError> {
    let mut changes = Vec::new();
    {
        let work = tx.work();
        let seq = tx.seq(seq_id)?;
        for &id in touched {
            let (Some(clip), Some(loc)) = (work.clip(id), work.find_clip(id)) else {
                continue;
            };
            let Some(track) = seq.track_at(loc.slot) else {
                continue;
            };
            let want = attachment_for(seq, loc.slot, track, clip, work.library());
            if want != clip.attach {
                let mut after = (**clip).clone();
                after.attach = want;
                changes.push(Change::ReplaceClip {
                    seq: seq_id,
                    track: track.id,
                    before: Arc::clone(clip),
                    after: Arc::new(after),
                });
            }
        }
    }
    for change in changes {
        tx.push(change)?;
    }
    Ok(())
}

/// Recomputes the attachment of every clip on the unlocked non-main tracks
/// of `seq` from its position (after an op that moved everything, such as
/// conform).
pub(crate) fn reattach_all(tx: &mut Tx<'_>, seq_id: SeqId) -> Result<(), EditError> {
    let ids: Vec<ClipId> = tx
        .seq(seq_id)?
        .tracks()
        .filter(|(slot, t)| !slot.is_main() && !t.is_locked())
        .flat_map(|(_, t)| t.clips().iter().map(|c| c.id))
        .collect();
    reattach(tx, seq_id, &ids)
}
