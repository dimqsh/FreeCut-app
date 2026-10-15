//! Trimming clip edges (TL-020), with linked partners (TL-057).

use std::collections::BTreeSet;
use std::sync::Arc;

use super::{Resolved, Unavailable, resolve_clips};
use crate::edit::policy::{self, MainItem};
use crate::edit::tx::Tx;
use crate::edit::{Change, EditError, Label, Notice, OpKind, Plan};
use crate::ids::{ClipId, SeqId};
use crate::model::{Clip, ClipContent, Track};
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
///
/// `partners` are linked clips (TL-057) whose same edge trims with the clip
/// in the same entry ([`crate::select::linked_edges`] finds them). They
/// lose or gain the same quantized amount. When any member ripples on the
/// magnetic main track, every member trims its head in place (it keeps its
/// start), so the pair stays in sync; later clips on a partner's own track
/// move only with `ripple`. A partner the change doesn't fit (locked, too
/// short, past its source, or colliding) is left as it is and reported in
/// [`Notice::Skipped`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrimReq {
    pub seq: SeqId,
    pub clip: ClipId,
    pub edge: Edge,
    pub delta: Tick,
    pub ripple: bool,
    pub partners: Vec<ClipId>,
}

/// One clip of a trim: the dragged clip or a partner.
struct Member {
    target: Resolved,
    track: Arc<Track>,
    magnetic: bool,
}

pub(crate) fn trim(mut tx: Tx<'_>, req: &TrimReq) -> Result<Plan, EditError> {
    let label = Label {
        op: OpKind::TrimClip,
        count: 1,
    };
    let target = resolve_clips(&tx, req.seq, &[req.clip])?
        .pop()
        .ok_or(EditError::StaleClip(req.clip))?;
    let seq = tx.seq(req.seq)?.clone();
    let magnet = seq.flags.main_magnet;
    let member = |target: Resolved| -> Result<Member, EditError> {
        let track = seq
            .track_at(target.loc.slot)
            .cloned()
            .ok_or(EditError::StaleTrack(target.loc.track))?;
        let magnetic = target.loc.slot.is_main() && magnet;
        Ok(Member {
            target,
            track,
            magnetic,
        })
    };
    let main = member(target)?;
    policy::require_unlocked(&main.track, main.target.loc.slot)?;

    let mut skipped: Vec<(ClipId, Unavailable)> = Vec::new();
    let mut partners: Vec<Member> = Vec::new();
    let mut seen = BTreeSet::from([req.clip]);
    for &id in &req.partners {
        if !seen.insert(id) {
            continue;
        }
        // A partner that is gone or lives elsewhere is not part of the trim.
        let Ok(found) = resolve_clips(&tx, req.seq, &[id]) else {
            continue;
        };
        for p in found {
            let p = member(p)?;
            if p.track.is_locked() {
                skipped.push((id, Unavailable::LockedTrack));
            } else {
                partners.push(p);
            }
        }
    }
    let in_place = req.ripple || main.magnetic || partners.iter().any(|p| p.magnetic);

    let rate = tx.rate();
    let clip = &main.target.clip;
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

    let main_after = trimmed(&tx, &main, req, d, in_place)?;
    let mut done = vec![(main, main_after)];
    for p in partners {
        match trimmed(&tx, &p, req, d, in_place) {
            Ok(after) => done.push((p, after)),
            Err(_) => skipped.push((p.target.clip.id, Unavailable::NoRoom)),
        }
    }

    let mut relayout_main = false;
    for (m, after) in &done {
        let before = &m.target.clip;
        let old_end = before.end();
        let end_change = after.end().checked_sub(old_end)?;
        tx.push(Change::ReplaceClip {
            seq: req.seq,
            track: m.track.id,
            before: Arc::clone(before),
            after: Arc::new(after.clone()),
        })?;
        if m.magnetic {
            relayout_main = true;
        } else if req.ripple {
            let later = m
                .track
                .clips()
                .iter()
                .filter(|c| c.id != before.id && c.start() >= old_end)
                .map(|c| (Arc::clone(c), end_change))
                .collect();
            policy::shift(&mut tx, req.seq, m.track.id, later)?;
        }
    }
    if relayout_main {
        let order = policy::main_layout(tx.seq(req.seq)?)?
            .into_iter()
            .map(|(id, _, _)| MainItem::Existing(id))
            .collect();
        policy::layout_main(&mut tx, req.seq, order)?;
    }
    if !skipped.is_empty() {
        tx.notice(Notice::Skipped(skipped));
    }
    tx.finish(Label {
        count: u32::try_from(done.len()).unwrap_or(u32::MAX),
        ..label
    })
}

/// Member `m` with `req.edge` moved by the quantized `d`: its head in
/// place when `in_place`. Refused when it would be shorter than a frame,
/// run past its source, start before 0, or (on a track that doesn't ripple)
/// collide with its neighbours.
fn trimmed(
    tx: &Tx<'_>,
    m: &Member,
    req: &TrimReq,
    d: Tick,
    in_place: bool,
) -> Result<Clip, EditError> {
    let clip = &m.target.clip;
    let rate = tx.rate();
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
    let after = match (req.edge, in_place) {
        (Edge::In, false) => clip.with_head_trimmed(d)?,
        (Edge::In, true) => clip.with_head_trimmed_in_place(d)?,
        (Edge::Out, _) => clip.with_tail_trimmed(d)?,
    };
    // The magnetic main track closes up and a ripple moves later clips;
    // anywhere else the neighbours limit the trim.
    if !m.magnetic && !req.ripple {
        let track = &m.track;
        let pos = track
            .index_of(clip.start(), clip.id)
            .ok_or(EditError::StaleClip(clip.id))?;
        let prev_end = pos
            .checked_sub(1)
            .and_then(|i| track.clips().get(i))
            .map_or(Tick::ZERO, |c| c.end());
        let next_start = track.clips().get(pos + 1).map(|c| c.start());
        if after.start().is_negative() {
            return Err(EditError::BeforeZero);
        }
        if after.start() < prev_end || next_start.is_some_and(|n| after.end() > n) {
            return Err(EditError::Collision { track: track.id });
        }
    }
    Ok(after)
}
