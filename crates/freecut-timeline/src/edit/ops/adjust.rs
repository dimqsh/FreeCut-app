//! Adding an adjustment clip (CL-006, CL-044): a clip with no media whose
//! colour stack grades everything on the tracks below it for as long as it
//! spans.
//!
//! **Span.** The union of the listed clips' ranges, or the default length
//! (3 s on the frame grid) from the playhead.
//!
//! **Placement** (CL-006, mirroring effect layers): the lowest unlocked,
//! visible `Adjustment` track above every track with content in the span
//! that is free there; otherwise a new `Adjustment` track directly above
//! the topmost track with content in the span (strict layering keeps it in
//! the upper band, above every video track). The attached-items pass
//! (TL-029) runs as after any insert, so the new clip follows the main
//! track like the other overlay kinds.

use std::collections::BTreeSet;
use std::sync::Arc;

use super::resolve_clips;
use crate::edit::policy;
use crate::edit::tx::Tx;
use crate::edit::{Change, EditError, Label, Notice, OpKind, Plan};
use crate::ids::{ClipId, SeqId};
use crate::model::{Clip, ClipContent, ColorStack, GeneratedKind, TrackKind};
use crate::time::{FrameRate, TICKS_PER_SECOND, Tick, TimeError, TimeRange};

/// Where a new adjustment clip spans (CL-006's default span).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AdjustSpan {
    /// The union time range of these clips (the selection).
    Selection(Vec<ClipId>),
    /// The default length starting at this time (the playhead).
    AtPlayhead(Tick),
}

/// The length of an adjustment clip added at the playhead before rounding
/// to the frame grid: 3 s (CL-006, FreeCut default).
pub const DEFAULT_ADJUSTMENT_DURATION: Tick = Tick::from_raw(3 * TICKS_PER_SECOND);

/// [`DEFAULT_ADJUSTMENT_DURATION`] on `rate`'s frame grid, at least one
/// frame.
pub fn default_adjustment_duration(rate: FrameRate) -> Result<Tick, TimeError> {
    Ok(rate
        .quantize(DEFAULT_ADJUSTMENT_DURATION)?
        .max(rate.frame_ticks()))
}

pub(crate) fn add_adjustment_clip(
    mut tx: Tx<'_>,
    seq_id: SeqId,
    span: &AdjustSpan,
    color: Option<&Arc<ColorStack>>,
) -> Result<Plan, EditError> {
    let rate = tx.rate();
    let range = match span {
        AdjustSpan::Selection(ids) => {
            let clips = resolve_clips(&tx, seq_id, ids)?;
            let start = clips.iter().map(|r| r.clip.start()).min();
            let end = clips.iter().map(|r| r.clip.end()).max();
            let (Some(start), Some(end)) = (start, end) else {
                return Err(EditError::NothingSelected);
            };
            let start = rate.quantize(start)?.max(Tick::ZERO);
            let end = rate
                .quantize(end)?
                .max(start.checked_add(rate.frame_ticks())?);
            TimeRange::from_start_end(start, end)?
        }
        AdjustSpan::AtPlayhead(t) => {
            let start = rate.quantize(*t)?.max(Tick::ZERO);
            TimeRange::new(start, default_adjustment_duration(rate)?)?
        }
    };
    let id = tx.ids.clip()?;
    let mut clip = Clip::new(
        id,
        range,
        ClipContent::Generated {
            kind: GeneratedKind::Adjustment,
        },
    );
    clip.color = color.and_then(|c| ColorStack::stored(c.clamped()));

    let seq = tx.seq(seq_id)?;
    let visual = seq.visual_tracks();
    let top = visual
        .iter()
        .rposition(|t| t.clips_in_range(range).next().is_some());
    let above = |i: usize| top.is_none_or(|top| i > top);
    let existing = visual
        .iter()
        .enumerate()
        .find(|(i, t)| {
            above(*i)
                && t.kind == TrackKind::Adjustment
                && !t.is_locked()
                && !t.header.flags.hidden
                && policy::fits(t, &[range], &BTreeSet::new())
        })
        .map(|(_, t)| t.id);
    let track = match existing {
        Some(track) => track,
        None => {
            let slot = policy::slot_for_new_track(seq, TrackKind::Adjustment, top.map(|i| i + 1));
            let track = policy::add_track_at(&mut tx, seq_id, TrackKind::Adjustment, slot)?;
            tx.notice(Notice::CreatedTrack(track));
            track
        }
    };
    tx.push(Change::InsertClip {
        seq: seq_id,
        track,
        clip: Arc::new(clip),
    })?;
    tx.select(vec![id]);
    tx.finish(Label {
        op: OpKind::AddAdjustmentClip,
        count: 1,
    })
}
