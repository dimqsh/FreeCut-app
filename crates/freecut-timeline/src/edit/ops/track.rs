//! Track structure, flags and the track stage (TL-005, TL-006, TL-008,
//! TL-010, AU-013).

use std::sync::Arc;

use crate::edit::policy::{self, MainItem};
use crate::edit::tx::Tx;
use crate::edit::{Change, EditError, Label, Notice, OpKind, Plan};
use crate::ids::{SeqId, TrackId};
use crate::model::{GainDb, Lane, Pan, TrackAudio, TrackHeader, TrackKind};

/// Where to add a track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackPosition {
    Above(TrackId),
    Below(TrackId),
    /// Top of the kind's band, or the bottom of the audio tracks.
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddTrackReq {
    pub seq: SeqId,
    pub kind: TrackKind,
    pub position: TrackPosition,
}

const MAX_NAME_CHARS: usize = 64;

pub(crate) fn add_track(mut tx: Tx<'_>, req: &AddTrackReq) -> Result<Plan, EditError> {
    let seq = tx.seq(req.seq)?;
    let desired = match req.position {
        TrackPosition::Default => None,
        TrackPosition::Above(t) | TrackPosition::Below(t) => {
            let (slot, _) = seq.find_track(t).ok_or(EditError::StaleTrack(t))?;
            let same_lane = (slot.lane == Lane::Audio) == req.kind.is_audio();
            // Visual lanes grow upward, audio lanes downward.
            let above = matches!(req.position, TrackPosition::Above(_));
            same_lane.then_some(match (slot.lane, above) {
                (Lane::Visual, true) | (Lane::Audio, false) => slot.index + 1,
                (Lane::Visual, false) | (Lane::Audio, true) => slot.index,
            })
        }
    };
    let slot = policy::slot_for_new_track(seq, req.kind, desired);
    policy::add_track_at(&mut tx, req.seq, req.kind, slot)?;
    tx.finish(Label {
        op: OpKind::AddTrack,
        count: 1,
    })
}

pub(crate) fn remove_track(mut tx: Tx<'_>, seq: SeqId, track: TrackId) -> Result<Plan, EditError> {
    let (slot, snapshot) = tx
        .seq(seq)?
        .find_track(track)
        .map(|(s, t)| (s, Arc::clone(t)))
        .ok_or(EditError::StaleTrack(track))?;
    if slot.is_main() {
        return Err(EditError::MainTrackRemoval);
    }
    if snapshot.is_locked() && !snapshot.is_empty() {
        return Err(EditError::TrackLocked { track });
    }
    tx.push(Change::RemoveTrack {
        seq,
        slot,
        track: snapshot,
    })?;
    tx.finish(Label {
        op: OpKind::RemoveTrack,
        count: 1,
    })
}

/// Trims the name, strips control characters and caps its length; an empty
/// name reverts to the default (TL-010).
fn clean_name(name: Option<&str>) -> Option<String> {
    let cleaned: String = name?
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(MAX_NAME_CHARS)
        .collect();
    (!cleaned.is_empty()).then_some(cleaned)
}

pub(crate) fn set_track_header(
    mut tx: Tx<'_>,
    seq: SeqId,
    track: TrackId,
    header: &TrackHeader,
) -> Result<Plan, EditError> {
    let (_, current) = tx
        .seq(seq)?
        .find_track(track)
        .ok_or(EditError::StaleTrack(track))?;
    let before = current.header.clone();
    let after = TrackHeader {
        name: clean_name(header.name.as_deref()),
        flags: header.flags,
    };
    if before != after {
        tx.push(Change::SetTrackHeader {
            seq,
            track,
            before,
            after,
        })?;
    }
    tx.finish(Label {
        op: OpKind::SetTrackHeader,
        count: 1,
    })
}

/// Toggles the magnetic main track. Switching it on closes every gap on the
/// main track in the same undo step (TL-005). A locked main track refuses
/// only a switch-on that would move clips; switching off moves nothing.
pub(crate) fn set_magnet(mut tx: Tx<'_>, seq: SeqId, on: bool) -> Result<Plan, EditError> {
    let s = tx.seq(seq)?;
    let before = s.flags;
    if before.main_magnet == on {
        return tx.finish(Label {
            op: OpKind::SetMagnet,
            count: 0,
        });
    }
    let main = policy::main_track(s)?;
    let gaps = main.gaps().count();
    if on && gaps > 0 && main.is_locked() {
        return Err(EditError::MainTrackLocked);
    }
    let mut after = before;
    after.main_magnet = on;
    tx.push(Change::SetSeqFlags { seq, before, after })?;
    if on {
        let order = main
            .clips()
            .iter()
            .map(|c| MainItem::Existing(c.id))
            .collect();
        policy::layout_main(&mut tx, seq, order)?;
        if gaps > 0 {
            tx.notice(Notice::ClosedGaps(u32::try_from(gaps).unwrap_or(u32::MAX)));
        }
    }
    tx.finish(Label {
        op: OpKind::SetMagnet,
        count: 1,
    })
}

/// Sets the volume of a track that carries sound (AU-013), clamped to the
/// track range.
pub(crate) fn set_track_gain(
    tx: Tx<'_>,
    seq: SeqId,
    track: TrackId,
    gain: GainDb,
) -> Result<Plan, EditError> {
    set_track_audio(tx, seq, track, OpKind::SetTrackGain, |a| TrackAudio {
        gain: TrackAudio::clamp_gain(gain),
        ..a
    })
}

/// Sets the pan of a track that carries sound (AU-013).
pub(crate) fn set_track_pan(
    tx: Tx<'_>,
    seq: SeqId,
    track: TrackId,
    pan: Pan,
) -> Result<Plan, EditError> {
    set_track_audio(tx, seq, track, OpKind::SetTrackPan, |a| TrackAudio {
        pan,
        ..a
    })
}

/// Changes the track stage of `track`. Like the other track header
/// controls (mute, TL-008), it works on locked tracks: a lock protects the
/// clips. Tracks that carry no sound refuse it; an unchanged value records
/// nothing.
fn set_track_audio(
    mut tx: Tx<'_>,
    seq: SeqId,
    track: TrackId,
    op: OpKind,
    f: impl FnOnce(TrackAudio) -> TrackAudio,
) -> Result<Plan, EditError> {
    let (_, current) = tx
        .seq(seq)?
        .find_track(track)
        .ok_or(EditError::StaleTrack(track))?;
    if !current.kind.carries_sound() {
        return Err(EditError::TrackHasNoSound { track });
    }
    let before = current.mix;
    let after = f(before);
    let count = u32::from(after != before);
    if after != before {
        tx.push(Change::SetTrackAudio {
            seq,
            track,
            before,
            after,
        })?;
    }
    tx.finish(Label { op, count })
}
