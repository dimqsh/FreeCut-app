//! Trimming clip edges (TL-020).

use std::sync::Arc;

use super::resolve_clips;
use crate::edit::policy::{self, MainItem};
use crate::edit::tx::Tx;
use crate::edit::{Change, EditError, Label, OpKind, Plan};
use crate::ids::{ClipId, SeqId};
use crate::model::ClipContent;
use crate::time::Tick;

/// A clip edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Edge {
    In,
    Out,
}

/// Moves one edge of a clip by `delta` (positive = later).
///
/// A plain trim leaves other clips alone and is limited by the neighbors. A
/// ripple trim keeps the clip's start, and later clips on the track follow
/// the change in length. Trims on the magnetic main track always ripple.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrimReq {
    pub seq: SeqId,
    pub clip: ClipId,
    pub edge: Edge,
    pub delta: Tick,
    pub ripple: bool,
}

pub(crate) fn trim(mut tx: Tx<'_>, req: &TrimReq) -> Result<Plan, EditError> {
    let label = Label {
        op: OpKind::TrimClip,
        count: 1,
    };
    let target = resolve_clips(&tx, req.seq, &[req.clip])?
        .pop()
        .ok_or(EditError::StaleClip(req.clip))?;
    let seq = tx.seq(req.seq)?;
    let track = seq
        .track_at(target.loc.slot)
        .cloned()
        .ok_or(EditError::StaleTrack(target.loc.track))?;
    policy::require_unlocked(&track, target.loc.slot)?;
    let magnetic_main = target.loc.slot.is_main() && seq.flags.main_magnet;
    let ripple = req.ripple || magnetic_main;

    let rate = tx.rate();
    let clip = &target.clip;
    let old_edge = match req.edge {
        Edge::In => clip.start(),
        Edge::Out => clip.end(),
    };
    let d = rate
        .quantize(old_edge.checked_add(req.delta)?)?
        .checked_sub(old_edge)?;
    // An edge off the grid (after a rate change) must not move unless asked
    // to, and rounding never moves it against the drag.
    if req.delta == Tick::ZERO || d.raw().signum() * req.delta.raw().signum() <= 0 {
        return tx.finish(Label { count: 0, ..label });
    }

    let new_duration = match req.edge {
        Edge::In => clip.duration().checked_sub(d)?,
        Edge::Out => clip.duration().checked_add(d)?,
    };
    if new_duration < rate.frame_ticks() {
        return Err(EditError::TooShort);
    }
    if let ClipContent::Media { asset, source, .. } = &clip.content {
        let past_start = req.edge == Edge::In && source.start().checked_add(d)?.is_negative();
        let past_end = if req.edge == Edge::Out && d.is_positive() {
            let len = tx
                .media_duration(*asset)
                .ok_or(EditError::UnknownMedia(*asset))?;
            source.end().checked_add(d)? > len
        } else {
            false
        };
        if past_start || past_end {
            return Err(EditError::SourceLimit { clip: clip.id });
        }
    }

    let trimmed = match (req.edge, ripple) {
        (Edge::In, false) => clip.with_head_trimmed(d)?,
        (Edge::In, true) => clip.with_head_trimmed_in_place(d)?,
        (Edge::Out, _) => clip.with_tail_trimmed(d)?,
    };

    if !ripple {
        let pos = track
            .index_of(clip.start(), clip.id)
            .ok_or(EditError::StaleClip(clip.id))?;
        let prev_end = pos
            .checked_sub(1)
            .and_then(|i| track.clips().get(i))
            .map_or(Tick::ZERO, |c| c.end());
        let next_start = track.clips().get(pos + 1).map(|c| c.start());
        if trimmed.start().is_negative() {
            return Err(EditError::BeforeZero);
        }
        if trimmed.start() < prev_end || next_start.is_some_and(|n| trimmed.end() > n) {
            return Err(EditError::Collision { track: track.id });
        }
    }

    let old_end = clip.end();
    let end_change = trimmed.end().checked_sub(old_end)?;
    tx.push(Change::ReplaceClip {
        seq: req.seq,
        track: track.id,
        before: Arc::clone(clip),
        after: Arc::new(trimmed),
    })?;

    if magnetic_main {
        let order = policy::main_layout(tx.seq(req.seq)?)?
            .into_iter()
            .map(|(id, _, _)| MainItem::Existing(id))
            .collect();
        policy::layout_main(&mut tx, req.seq, order)?;
    } else if ripple {
        let later = track
            .clips()
            .iter()
            .filter(|c| c.id != clip.id && c.start() >= old_end)
            .map(|c| (Arc::clone(c), end_change))
            .collect();
        policy::shift(&mut tx, req.seq, track.id, later)?;
    }
    tx.finish(label)
}
