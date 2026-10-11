//! Delete and ripple delete (TL-025).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::resolve_clips;
use crate::edit::policy::{self, MainItem};
use crate::edit::tx::Tx;
use crate::edit::{Change, EditError, Label, OpKind, Plan};
use crate::ids::{ClipId, GroupId, SeqId};
use crate::model::{Clip, TrackSlot};
use crate::time::Tick;

/// Removes clips (and the rest of their groups). A plain delete leaves gaps,
/// except on the magnetic main track, which always closes up. A ripple
/// delete closes the removed spans on every affected track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteReq {
    pub seq: SeqId,
    pub clips: Vec<ClipId>,
    pub ripple: bool,
}

pub(crate) fn delete(mut tx: Tx<'_>, req: &DeleteReq) -> Result<Plan, EditError> {
    let op = if req.ripple {
        OpKind::RippleDeleteClips
    } else {
        OpKind::DeleteClips
    };
    let count = delete_in(&mut tx, req)?;
    if count > 0 {
        tx.select(Vec::new());
    }
    tx.finish(Label { op, count })
}

/// Deletes inside an open transaction; returns how many clips went.
pub(super) fn delete_in(tx: &mut Tx<'_>, req: &DeleteReq) -> Result<u32, EditError> {
    let groups: BTreeSet<GroupId> = req
        .clips
        .iter()
        .filter_map(|id| tx.work().clip(*id))
        .filter_map(|c| c.group)
        .collect();
    let seq = tx.seq(req.seq)?;
    let mut ids = req.clips.clone();
    if !groups.is_empty() {
        for (_, track) in seq.tracks() {
            ids.extend(
                track
                    .clips()
                    .iter()
                    .filter(|c| c.group.is_some_and(|g| groups.contains(&g)))
                    .map(|c| c.id),
            );
        }
    }
    let targets = resolve_clips(tx, req.seq, &ids)?;
    if targets.is_empty() {
        return Ok(0);
    }
    let magnet = seq.flags.main_magnet;

    let mut by_track: BTreeMap<TrackSlot, Vec<Arc<Clip>>> = BTreeMap::new();
    for t in &targets {
        let track = seq
            .track_at(t.loc.slot)
            .ok_or(EditError::StaleTrack(t.loc.track))?;
        policy::require_unlocked(track, t.loc.slot)?;
        by_track
            .entry(t.loc.slot)
            .or_default()
            .push(Arc::clone(&t.clip));
    }
    let tracks: Vec<_> = by_track
        .keys()
        .map(|slot| seq.track_at(*slot).map(|t| (*slot, t.id)))
        .collect::<Option<_>>()
        .ok_or(EditError::TrackOutOfRange)?;

    for t in &targets {
        tx.push(Change::RemoveClip {
            seq: req.seq,
            track: t.loc.track,
            clip: Arc::clone(&t.clip),
        })?;
    }

    for (slot, track_id) in tracks {
        if slot.is_main() && magnet {
            let order = policy::main_layout(tx.seq(req.seq)?)?
                .into_iter()
                .map(|(id, _, _)| MainItem::Existing(id))
                .collect();
            policy::layout_main(tx, req.seq, order)?;
        } else if req.ripple {
            let removed = &by_track[&slot];
            let track = tx
                .seq(req.seq)?
                .find_track(track_id)
                .map(|(_, t)| Arc::clone(t))
                .ok_or(EditError::StaleTrack(track_id))?;
            // One sweep: survivors and removed clips are both in time order,
            // and each survivor closes up by the length removed before it.
            let mut removed: Vec<&Arc<Clip>> = removed.iter().collect();
            removed.sort_by_key(|c| c.start());
            let mut moves = Vec::new();
            let mut closed = Tick::ZERO;
            let mut next = removed.iter().peekable();
            for survivor in track.clips() {
                while let Some(r) = next.next_if(|r| r.end() <= survivor.start()) {
                    closed = closed.checked_add(r.duration())?;
                }
                moves.push((Arc::clone(survivor), closed.checked_neg()?));
            }
            policy::shift(tx, req.seq, track_id, moves)?;
        }
    }
    Ok(u32::try_from(targets.len()).unwrap_or(u32::MAX))
}
