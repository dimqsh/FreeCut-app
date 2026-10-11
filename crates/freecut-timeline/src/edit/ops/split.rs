//! Splitting clips (TL-023).

use std::sync::Arc;

use super::{Resolved, resolve_clips};
use crate::edit::policy;
use crate::edit::tx::Tx;
use crate::edit::{Change, EditError, Label, OpKind, Plan};
use crate::ids::{ClipId, SeqId};
use crate::model::ClipLoc;
use crate::time::Tick;

/// Which clips to split.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplitTargets {
    /// These clips (refused if any is on a locked track).
    Clips(Vec<ClipId>),
    /// Every clip under the split point on every unlocked track.
    AllTracks,
}

/// Splits clips at `at`. The left part keeps the clip's id, the right part
/// gets a new one and is selected afterward. Clips whose edge is at `at`
/// aren't split.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitReq {
    pub seq: SeqId,
    pub at: Tick,
    pub targets: SplitTargets,
}

pub(crate) fn split(mut tx: Tx<'_>, req: &SplitReq) -> Result<Plan, EditError> {
    let at = tx.rate().quantize(req.at)?;
    let targets: Vec<Resolved> = match &req.targets {
        SplitTargets::Clips(ids) => {
            let resolved = resolve_clips(&tx, req.seq, ids)?;
            let seq = tx.seq(req.seq)?;
            for r in &resolved {
                let track = seq
                    .track_at(r.loc.slot)
                    .ok_or(EditError::StaleTrack(r.loc.track))?;
                policy::require_unlocked(track, r.loc.slot)?;
            }
            resolved
        }
        SplitTargets::AllTracks => {
            let seq = tx.seq(req.seq)?;
            seq.tracks()
                .filter(|(_, t)| !t.is_locked())
                .filter_map(|(slot, t)| {
                    t.clip_at(at).map(|c| Resolved {
                        loc: ClipLoc {
                            seq: req.seq,
                            track: t.id,
                            slot,
                        },
                        clip: Arc::clone(c),
                    })
                })
                .collect()
        }
    };

    let frame = tx.rate().frame_ticks();
    let mut rights = Vec::new();
    let mut unsplit = Vec::new();
    for target in targets {
        // Both parts must be at least one frame (TL-011, TL-023); after a
        // rate change an edge can sit less than a frame from the grid point.
        let long_enough = at.checked_sub(target.clip.start())? >= frame
            && target.clip.end().checked_sub(at)? >= frame;
        if !target.clip.range.strictly_contains(at) || !long_enough {
            unsplit.push(target.clip.id);
            continue;
        }
        let right_id = tx.ids.clip()?;
        let (left, right) = target.clip.split_at(at, right_id)?;
        tx.push(Change::ReplaceClip {
            seq: req.seq,
            track: target.loc.track,
            before: Arc::clone(&target.clip),
            after: Arc::new(left),
        })?;
        tx.push(Change::InsertClip {
            seq: req.seq,
            track: target.loc.track,
            clip: Arc::new(right),
        })?;
        rights.push(right_id);
    }
    let count = u32::try_from(rights.len()).unwrap_or(u32::MAX);
    if !rights.is_empty() {
        // The right parts are selected; requested clips that weren't split
        // stay selected (TL-015, TL-023).
        if matches!(req.targets, SplitTargets::Clips(_)) {
            rights.extend(unsplit);
        }
        tx.select(rights);
    }
    tx.finish(Label {
        op: OpKind::SplitClips,
        count,
    })
}
