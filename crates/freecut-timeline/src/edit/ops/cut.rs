//! Cut away the part before or after the playhead (TL-026).

use std::sync::Arc;

use super::{Resolved, resolve_clips};
use crate::edit::policy::{self, MainItem};
use crate::edit::tx::Tx;
use crate::edit::{Change, EditError, Label, Notice, OpKind, Plan};
use crate::ids::{ClipId, SeqId};
use crate::model::{Clip, ClipLoc, Document, Sequence, TrackSlot};
use crate::time::{FrameRate, Tick};

/// Which clips a cut-away acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrimTargets {
    /// These clips (the selection, with linked partners when the app
    /// expands them, TL-057).
    Clips(Vec<ClipId>),
    /// The main-track clip under the playhead (nothing selected).
    MainUnderPlayhead,
}

/// Which side of the playhead goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cut {
    /// Cut away the part before the playhead (`Q`).
    Head,
    /// Cut away the part after the playhead (`W`).
    Tail,
}

/// Why an op can't act on a clip now (TL-054). Commands show it as the
/// disabled reason or count it among the skipped clips (TL-053).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Unavailable {
    /// Nothing selected and no main-track clip under the playhead.
    NoTarget,
    /// The clip is on a locked track.
    LockedTrack,
    /// The playhead isn't strictly inside the clip (or a part would be
    /// shorter than a frame).
    PlayheadOutside,
}

/// Whether a cut-away at `at` would act on anything: `Ok` when at least one
/// target can be cut, else the first target's reason (the shared check for
/// the toolbar buttons, menus and keys).
pub fn trim_to_playhead_available(
    doc: &Document,
    seq: SeqId,
    targets: &TrimTargets,
    at: Tick,
) -> Result<(), Unavailable> {
    let Some(s) = doc.sequence(seq) else {
        return Err(Unavailable::NoTarget);
    };
    let rate = doc.settings().frame_rate;
    let at = rate
        .quantize(at)
        .map_err(|_| Unavailable::PlayheadOutside)?;
    let found: Vec<(ClipLoc, Arc<Clip>)> = match targets {
        TrimTargets::Clips(ids) => ids
            .iter()
            .filter_map(|id| {
                let loc = doc.find_clip(*id).filter(|l| l.seq == seq)?;
                Some((loc, Arc::clone(doc.clip(*id)?)))
            })
            .collect(),
        TrimTargets::MainUnderPlayhead => main_under(s, seq, at).into_iter().collect(),
    };
    let mut first = None;
    for (loc, clip) in &found {
        match eligibility(s, loc, clip, at, rate) {
            Ok(()) => return Ok(()),
            Err(why) => {
                first.get_or_insert(why);
            }
        }
    }
    Err(first.unwrap_or(Unavailable::NoTarget))
}

fn main_under(s: &Sequence, seq: SeqId, at: Tick) -> Option<(ClipLoc, Arc<Clip>)> {
    let main = s.main_track()?;
    let clip = main.clip_at(at)?;
    Some((
        ClipLoc {
            seq,
            track: main.id,
            slot: TrackSlot::MAIN,
        },
        Arc::clone(clip),
    ))
}

/// TL-054 for a cut-away: unlocked, and the playhead strictly inside with at
/// least a frame on both sides.
fn eligibility(
    s: &Sequence,
    loc: &ClipLoc,
    clip: &Clip,
    at: Tick,
    rate: FrameRate,
) -> Result<(), Unavailable> {
    if s.track_at(loc.slot).is_none_or(|t| t.is_locked()) {
        return Err(Unavailable::LockedTrack);
    }
    let frame = rate.frame_ticks();
    let inside = clip.range.strictly_contains(at)
        && at.checked_sub(clip.start()).is_ok_and(|d| d >= frame)
        && clip.end().checked_sub(at).is_ok_and(|d| d >= frame);
    if inside {
        Ok(())
    } else {
        Err(Unavailable::PlayheadOutside)
    }
}

/// Cuts every eligible target at the playhead in one entry; ineligible ones
/// are skipped and reported ([`Notice::Skipped`]), and the edit is refused
/// with [`EditError::NothingToTrim`] when none is left.
///
/// On the magnetic main track the clip keeps its start and the track closes
/// up (a head cut also reports [`Notice::PlayheadTo`] the clip's start);
/// elsewhere the cut edge moves to the playhead, and with `ripple_others`
/// later clips on that track close up too. Clips linked to a rippled
/// main-track clip (TL-057) lose the same content in place, so they stay in
/// sync.
pub(crate) fn trim_to_playhead(
    mut tx: Tx<'_>,
    seq: SeqId,
    targets: &TrimTargets,
    at: Tick,
    cut: Cut,
    ripple_others: bool,
) -> Result<Plan, EditError> {
    let rate = tx.rate();
    let at = rate.quantize(at)?;
    let resolved: Vec<Resolved> = match targets {
        TrimTargets::Clips(ids) => resolve_clips(&tx, seq, ids)?,
        TrimTargets::MainUnderPlayhead => main_under(tx.seq(seq)?, seq, at)
            .map(|(loc, clip)| Resolved { loc, clip })
            .into_iter()
            .collect(),
    };
    let s = tx.seq(seq)?.clone();
    let magnet = s.flags.main_magnet;
    let mut eligible = Vec::new();
    let mut skipped = Vec::new();
    for r in resolved {
        match eligibility(&s, &r.loc, &r.clip, at, rate) {
            Ok(()) => eligible.push(r),
            Err(why) => skipped.push((r.clip.id, why)),
        }
    }
    if eligible.is_empty() {
        return Err(EditError::NothingToTrim);
    }
    // Links of a main-track clip that ripples: its partners keep in sync by
    // losing the same content in place.
    let main_links: Vec<_> = eligible
        .iter()
        .filter(|r| r.loc.slot.is_main() && magnet)
        .filter_map(|r| r.clip.link)
        .collect();
    let mut relayout_main = false;
    for r in &eligible {
        let clip = &r.clip;
        let main_ripple = r.loc.slot.is_main() && magnet;
        let in_place =
            main_ripple || ripple_others || clip.link.is_some_and(|l| main_links.contains(&l));
        let (after, removed) = match cut {
            Cut::Head => {
                let d = at.checked_sub(clip.start())?;
                let after = if in_place {
                    clip.with_head_trimmed_in_place(d)?
                } else {
                    clip.with_head_trimmed(d)?
                };
                (after, d)
            }
            Cut::Tail => {
                let d = clip.end().checked_sub(at)?;
                (clip.with_tail_trimmed(d.checked_neg()?)?, d)
            }
        };
        tx.push(Change::ReplaceClip {
            seq,
            track: r.loc.track,
            before: Arc::clone(clip),
            after: Arc::new(after),
        })?;
        if main_ripple {
            relayout_main = true;
            if cut == Cut::Head {
                tx.notice(Notice::PlayheadTo(clip.start()));
            }
        } else if ripple_others {
            let track = s
                .track_at(r.loc.slot)
                .ok_or(EditError::StaleTrack(r.loc.track))?;
            let back = removed.checked_neg()?;
            let later = track
                .clips()
                .iter()
                .filter(|c| c.id != clip.id && c.start() >= clip.end())
                .map(|c| (Arc::clone(c), back))
                .collect();
            policy::shift(&mut tx, seq, r.loc.track, later)?;
        }
    }
    if relayout_main {
        let order = policy::main_layout(tx.seq(seq)?)?
            .into_iter()
            .map(|(id, _, _)| MainItem::Existing(id))
            .collect();
        policy::layout_main(&mut tx, seq, order)?;
    }
    if !skipped.is_empty() {
        tx.notice(Notice::Skipped(skipped));
    }
    tx.finish(Label {
        op: OpKind::TrimToPlayhead(cut),
        count: u32::try_from(eligible.len()).unwrap_or(u32::MAX),
    })
}
