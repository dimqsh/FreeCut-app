//! Conform to frame grid (TL-001).

use std::sync::Arc;

use crate::edit::tx::Tx;
use crate::edit::{Change, EditError, Label, OpKind, Plan};
use crate::ids::SeqId;
use crate::model::ClipContent;
use crate::time::{Rounding, Tick};

/// Rounds every clip's start and end onto the sequence frame grid, as one
/// undoable step after a frame-rate change.
///
/// - Edges go to the nearest frame (ties to the earlier one).
/// - A clip keeps at least one frame. If that, or rounding, would make it
///   overlap the next clip, the next clip moves along, so the track is
///   rippled rather than overwritten.
/// - Media clips never grow past the end of their source.
/// - The magnetic main track stays contiguous from 0.
/// - Locked tracks are left alone.
/// - Attached items are re-anchored where they land rather than following
///   their owners' sub-frame moves (TL-029).
pub(crate) fn conform(mut tx: Tx<'_>, seq_id: SeqId) -> Result<Plan, EditError> {
    let count = conform_in(&mut tx, seq_id)?;
    tx.finish(Label {
        op: OpKind::Conform,
        count,
    })
}

/// Conforms inside an open transaction; returns how many clips changed.
pub(super) fn conform_in(tx: &mut Tx<'_>, seq_id: SeqId) -> Result<u32, EditError> {
    let seq = tx.seq(seq_id)?.clone();
    let rate = tx.rate();
    let frame = rate.frame_ticks();
    let mut count: u32 = 0;
    for (slot, track) in seq.tracks() {
        if track.is_locked() {
            continue;
        }
        let magnetic = slot.is_main() && seq.flags.main_magnet;
        let mut prev_end = Tick::ZERO;
        for clip in track.clips() {
            let start = if magnetic {
                prev_end
            } else {
                rate.quantize(clip.start())?.max(prev_end)
            };
            let mut end = rate.quantize(clip.end())?.max(start.checked_add(frame)?);
            if let ClipContent::Media { asset, source, .. } = &clip.content {
                // Without a known length the clip may shrink but not grow.
                let available = tx.media_duration(*asset).unwrap_or(source.end());
                let room =
                    rate.round_to_grid(available.checked_sub(source.start())?, Rounding::Floor)?;
                if end.checked_sub(start)? > room {
                    if room < frame {
                        return Err(EditError::SourceLimit { clip: clip.id });
                    }
                    end = start.checked_add(room)?;
                }
            }
            if start != clip.start() || end != clip.end() {
                let resized = clip
                    .moved_to(start)?
                    .with_tail_trimmed(end.checked_sub(start)?.checked_sub(clip.duration())?)?;
                tx.push(Change::ReplaceClip {
                    seq: seq_id,
                    track: track.id,
                    before: Arc::clone(clip),
                    after: Arc::new(resized),
                })?;
                count = count.saturating_add(1);
            }
            prev_end = end;
        }
    }
    crate::edit::follow::reattach_all(tx, seq_id)?;
    tx.skip_follow();
    Ok(count)
}
