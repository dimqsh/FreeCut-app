//! Moving clips in time and across tracks (TL-018).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use super::{Resolved, resolve_clips};
use crate::edit::policy::{self, MainItem};
use crate::edit::tx::Tx;
use crate::edit::{Change, EditError, Label, Notice, OpKind, Plan};
use crate::ids::{ClipId, LinkId, SeqId, TrackId};
use crate::model::{Clip, Lane, Sequence, TrackKind, TrackSlot};
use crate::time::{Tick, TimeRange};

/// Moves clips by `delta` in time and `track_shift` tracks up (negative:
/// down). Relative offsets between the clips are preserved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveReq {
    pub seq: SeqId,
    pub clips: Vec<ClipId>,
    pub delta: Tick,
    pub track_shift: i32,
}

/// Where one moving clip ends up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dest {
    /// Onto the magnetic main track (position decided by the cut rule).
    MagneticMain,
    /// Onto an existing track.
    Track(TrackSlot),
    /// Onto a new track beyond the end of a lane; `k` orders several.
    NewTrack { lane: Lane, k: usize },
}

pub(crate) fn move_clips(mut tx: Tx<'_>, req: &MoveReq) -> Result<Plan, EditError> {
    let moving = resolve_clips(&tx, req.seq, &req.clips)?;
    if moving.is_empty() {
        return Err(EditError::NothingSelected);
    }
    // The block lands with its earliest clip on the frame grid (TL-001),
    // never moving against the drag.
    let reference = moving
        .iter()
        .map(|m| m.clip.start())
        .min()
        .unwrap_or(Tick::ZERO);
    let landed = tx.rate().quantize(reference.checked_add(req.delta)?)?;
    let mut delta = landed.checked_sub(reference)?;
    if req.delta == Tick::ZERO || delta.raw().signum() * req.delta.raw().signum() < 0 {
        delta = Tick::ZERO;
    }
    let label = Label {
        op: OpKind::MoveClips,
        count: u32::try_from(moving.len()).unwrap_or(u32::MAX),
    };
    if delta == Tick::ZERO && req.track_shift == 0 {
        return tx.finish(Label { count: 0, ..label });
    }
    let seq = tx.seq(req.seq)?.clone();
    let magnet = seq.flags.main_magnet;

    // Main-track members of a magnetic move must be adjacent, wherever they
    // go (TL-018).
    if magnet {
        let main = policy::main_track(&seq)?;
        let mut positions: Vec<usize> = moving
            .iter()
            .filter(|m| m.loc.slot.is_main())
            .filter_map(|m| main.index_of(m.clip.start(), m.clip.id))
            .collect();
        positions.sort_unstable();
        if positions.windows(2).any(|w| w[1] != w[0] + 1) {
            return Err(EditError::NonContiguousMain);
        }
    }

    let mut dests = Vec::with_capacity(moving.len());
    for m in &moving {
        let source = seq
            .track_at(m.loc.slot)
            .ok_or(EditError::StaleTrack(m.loc.track))?;
        policy::require_unlocked(source, m.loc.slot)?;
        let dest = destination(&seq, m, req.track_shift, magnet)?;
        dests.push(dest);
    }

    let moving_ids: BTreeSet<ClipId> = moving.iter().map(|m| m.clip.id).collect();
    let touches_main = moving
        .iter()
        .zip(&dests)
        .any(|(m, d)| *d == Dest::MagneticMain || (magnet && m.loc.slot.is_main()));

    // The magnetic main track's new order, and how far each clip landing
    // there moves: their link partners (TL-057) move by that, not by the
    // pointer delta, so they stay in sync.
    let main_order = if magnet && touches_main {
        Some(main_order(&tx, req.seq, &moving, &dests, delta)?)
    } else {
        None
    };
    let mut link_delta: BTreeMap<LinkId, Tick> = BTreeMap::new();
    let linked_to_main = moving
        .iter()
        .zip(&dests)
        .any(|(m, d)| *d == Dest::MagneticMain && m.clip.link.is_some());
    if let Some(order) = main_order.as_ref().filter(|_| linked_to_main) {
        let starts = main_starts(&tx, req.seq, order)?;
        for (m, dest) in moving.iter().zip(&dests) {
            if let (Dest::MagneticMain, Some(link), Some(start)) =
                (dest, m.clip.link, starts.get(&m.clip.id))
            {
                link_delta
                    .entry(link)
                    .or_insert(start.checked_sub(m.clip.start())?);
            }
        }
    }

    // New ranges for clips that don't land on the magnetic main track.
    let mut arrivals: BTreeMap<DestKey, Vec<Clip>> = BTreeMap::new();
    for (m, dest) in moving.iter().zip(&dests) {
        if *dest == Dest::MagneticMain {
            continue;
        }
        let own = m.clip.link.and_then(|l| link_delta.get(&l)).copied();
        let start = m.clip.start().checked_add(own.unwrap_or(delta))?;
        if start.is_negative() {
            return Err(EditError::BeforeZero);
        }
        arrivals
            .entry(DestKey::of(*dest))
            .or_default()
            .push(m.clip.moved_to(start)?);
    }

    // Collision checks against what stays put, and among the arrivals.
    let single = moving.len() == 1;
    let mut overflow_above: Option<(TrackSlot, TrackKind)> = None;
    for (key, clips) in &arrivals {
        let DestKey::Track(slot) = key else { continue };
        let track = seq.track_at(*slot).ok_or(EditError::TrackOutOfRange)?;
        let ranges: Vec<TimeRange> = clips.iter().map(|c| c.range).collect();
        if policy::fits(track, &ranges, &moving_ids) && !self_overlap(&ranges) {
            continue;
        }
        if single {
            overflow_above = Some((*slot, track.kind));
        } else {
            return Err(EditError::Collision { track: track.id });
        }
    }

    // 1. Lift every clip that leaves its track.
    let mut staying: BTreeMap<ClipId, Arc<Clip>> = BTreeMap::new();
    for (m, dest) in moving.iter().zip(&dests) {
        let stays = match dest {
            Dest::Track(slot) => *slot == m.loc.slot && overflow_above.is_none(),
            Dest::MagneticMain => m.loc.slot.is_main(),
            Dest::NewTrack { .. } => false,
        };
        if stays {
            staying.insert(m.clip.id, Arc::clone(&m.clip));
        } else {
            tx.push(Change::RemoveClip {
                seq: req.seq,
                track: m.loc.track,
                clip: Arc::clone(&m.clip),
            })?;
        }
    }

    // 2. Re-lay the magnetic main track.
    if let Some(order) = main_order {
        policy::layout_main(&mut tx, req.seq, order)?;
    }

    // 3. Land everything else.
    let mut new_tracks: BTreeMap<(Lane, usize), TrackId> = BTreeMap::new();
    for (key, clips) in arrivals {
        let track_id = match key {
            DestKey::Track(slot) => {
                let track = seq.track_at(slot).ok_or(EditError::TrackOutOfRange)?;
                if let Some((above, kind)) = overflow_above {
                    let s = policy::slot_next_to(tx.seq(req.seq)?, kind, above);
                    let id = policy::add_track_at(&mut tx, req.seq, kind, s)?;
                    tx.notice(Notice::CreatedTrack(id));
                    id
                } else {
                    track.id
                }
            }
            DestKey::NewTrack(lane, k) => {
                let kind = TrackKind::for_content(&clips[0].content);
                if clips
                    .iter()
                    .any(|c| TrackKind::for_content(&c.content) != kind)
                {
                    return Err(EditError::MixedKinds);
                }
                let s = policy::slot_for_new_track(tx.seq(req.seq)?, kind, None);
                let id = policy::add_track_at(&mut tx, req.seq, kind, s)?;
                new_tracks.insert((lane, k), id);
                id
            }
        };
        for clip in clips {
            let change = if let Some(before) = staying.get(&clip.id) {
                Change::ReplaceClip {
                    seq: req.seq,
                    track: track_id,
                    before: Arc::clone(before),
                    after: Arc::new(clip),
                }
            } else {
                Change::InsertClip {
                    seq: req.seq,
                    track: track_id,
                    clip: Arc::new(clip),
                }
            };
            tx.push(change)?;
        }
    }
    tx.select(moving.iter().map(|m| m.clip.id).collect());
    tx.finish(label)
}

/// Arrival groups, ordered so new tracks are created bottom-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DestKey {
    Track(TrackSlot),
    NewTrack(Lane, usize),
}

impl DestKey {
    fn of(dest: Dest) -> DestKey {
        match dest {
            Dest::Track(slot) => DestKey::Track(slot),
            Dest::NewTrack { lane, k } => DestKey::NewTrack(lane, k),
            // Callers skip magnetic-main destinations.
            Dest::MagneticMain => DestKey::Track(TrackSlot::MAIN),
        }
    }
}

fn destination(seq: &Sequence, m: &Resolved, shift: i32, magnet: bool) -> Result<Dest, EditError> {
    let lane = m.loc.slot.lane;
    let index = i64::try_from(m.loc.slot.index).map_err(|_| EditError::TrackOutOfRange)?;
    // Positive shifts move up: up the visual stack, and toward the main
    // track in the audio list.
    let target = match lane {
        Lane::Visual => index + i64::from(shift),
        Lane::Audio => index - i64::from(shift),
    };
    let len = i64::try_from(seq.lane(lane).len()).map_err(|_| EditError::TrackOutOfRange)?;
    if target < 0 {
        return Err(EditError::TrackOutOfRange);
    }
    if target >= len {
        let k = usize::try_from(target - len).map_err(|_| EditError::TrackOutOfRange)?;
        return Ok(Dest::NewTrack { lane, k });
    }
    let slot = TrackSlot {
        lane,
        index: usize::try_from(target).map_err(|_| EditError::TrackOutOfRange)?,
    };
    let track = seq.track_at(slot).ok_or(EditError::TrackOutOfRange)?;
    if !track.kind.accepts(&m.clip.content) {
        return Err(EditError::KindMismatch { track: track.id });
    }
    policy::require_unlocked(track, slot)?;
    Ok(if slot.is_main() && magnet {
        Dest::MagneticMain
    } else {
        Dest::Track(slot)
    })
}

fn self_overlap(ranges: &[TimeRange]) -> bool {
    let mut sorted: Vec<TimeRange> = ranges.to_vec();
    sorted.sort_by_key(|r| r.start());
    sorted.windows(2).any(|w| w[0].overlaps(w[1]))
}

/// The magnetic main track's order after a move. Clips that stay on it must
/// be adjacent; together with clips joining it they form one block that goes
/// in at the cut nearest the block's new start (TL-005, TL-018).
fn main_order(
    tx: &Tx<'_>,
    seq: SeqId,
    moving: &[Resolved],
    dests: &[Dest],
    delta: Tick,
) -> Result<Vec<MainItem>, EditError> {
    let main = policy::main_track(tx.seq(seq)?)?;
    let moving_ids: BTreeSet<ClipId> = moving.iter().map(|m| m.clip.id).collect();

    let mut block: Vec<(&Resolved, bool)> = moving
        .iter()
        .zip(dests)
        .filter(|(_, d)| **d == Dest::MagneticMain)
        .map(|(m, _)| (m, m.loc.slot.is_main()))
        .collect();
    // Adjacency of main-track members was checked by the caller.

    // The rest of the main track, laid out as it will be without the block.
    let mut remaining = Vec::new();
    let mut pos = Tick::ZERO;
    for clip in main.clips().iter().filter(|c| !moving_ids.contains(&c.id)) {
        remaining.push((clip.id, pos, clip.duration()));
        pos = pos.checked_add(clip.duration())?;
    }
    let mut order: Vec<MainItem> = remaining
        .iter()
        .map(|(id, _, _)| MainItem::Existing(*id))
        .collect();
    if !block.is_empty() {
        block.sort_by_key(|(m, _)| (m.clip.start().raw().saturating_add(delta.raw()), m.clip.id));
        let target = block[0].0.clip.start().checked_add(delta)?;
        let cut = policy::cut_index(&remaining, target);
        let items = block.into_iter().map(|(m, stays)| {
            if stays {
                MainItem::Existing(m.clip.id)
            } else {
                MainItem::New(Box::new((*m.clip).clone()))
            }
        });
        order.splice(cut..cut, items);
    }
    Ok(order)
}

/// Where each clip of `order` starts once the main track is laid out from
/// 0 (what [`policy::layout_main`] will do).
fn main_starts(
    tx: &Tx<'_>,
    seq: SeqId,
    order: &[MainItem],
) -> Result<BTreeMap<ClipId, Tick>, EditError> {
    let main = policy::main_track(tx.seq(seq)?)?;
    let lengths: HashMap<ClipId, Tick> =
        main.clips().iter().map(|c| (c.id, c.duration())).collect();
    let mut pos = Tick::ZERO;
    let mut out = BTreeMap::new();
    for item in order {
        let (id, duration) = match item {
            MainItem::Existing(id) => (*id, *lengths.get(id).ok_or(EditError::StaleClip(*id))?),
            MainItem::New(clip) => (clip.id, clip.duration()),
        };
        out.insert(id, pos);
        pos = pos.checked_add(duration)?;
    }
    Ok(out)
}
