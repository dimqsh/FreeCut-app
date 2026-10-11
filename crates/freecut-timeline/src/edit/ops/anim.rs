//! Choosing and changing clip animations (FX-018, FX-019, TX-022): one
//! slot on every listed visual clip, with the slot policy of each clip's
//! kind.

use super::props::try_update_clips;
use crate::edit::tx::Tx;
use crate::edit::{EditError, OpKind, Plan};
use crate::ids::{ClipId, SeqId};
use crate::model::{AnimationSpec, Clip, Slot, slot_policy};
use crate::motion::{pace_unit, with_paced_duration};
use crate::time::{FrameRate, Rounding, Tick};

/// Sets `slot` to `spec` (`None` clears it) on every listed visual clip;
/// audio clips are skipped and the edit is refused when none is visual.
/// On video, image and generated clips choosing Continuous clears Entrance
/// and Exit and the reverse; on text clips no other slot changes (FX-019).
pub(crate) fn set_clip_animation(
    tx: Tx<'_>,
    seq: SeqId,
    ids: &[ClipId],
    slot: Slot,
    spec: Option<&AnimationSpec>,
) -> Result<Plan, EditError> {
    let rate = tx.rate();
    try_update_clips(
        tx,
        seq,
        ids,
        OpKind::SetClipAnimation,
        EditError::NoVisualClip,
        |clip| {
            let Some(policy) = slot_policy(&clip.content) else {
                return Ok(None);
            };
            let spec = match spec {
                Some(s) => Some(fitted(s, clip, slot, rate)?),
                None => None,
            };
            let mut out = clip.clone();
            out.animations = clip.animations.with(slot, spec, policy);
            Ok(Some(out))
        },
    )
}

/// `spec` as stored on `clip` in `slot`: settings clamped (FX-030,
/// TX-023), Pace dropped on line and block units (TX-022); with Pace on a
/// text clip the In or Out length computed from
/// the text (TX-022); else In and Out snapped to frames, at least 0.1 s
/// and at most what the other slot leaves of the clip (FX-018), and a
/// Loop cycle within 0.1–10 s (FX-019). A clip without room for the
/// minimum still stores the minimum (TX-022: stored, played when there is
/// room). A Loop keeps the phase and the seams its clip already has
/// (FX-019's and TX-022's split rule); In and Out have none.
fn fitted(
    spec: &AnimationSpec,
    clip: &Clip,
    slot: Slot,
    rate: FrameRate,
) -> Result<AnimationSpec, EditError> {
    let mut s = spec.clamped();
    let min = rate
        .round_to_grid(AnimationSpec::MIN_DURATION, Rounding::Ceil)?
        .max(rate.frame_ticks());
    match slot {
        Slot::Continuous => {
            s.duration = s
                .duration
                .clamp(AnimationSpec::MIN_DURATION, AnimationSpec::MAX_CYCLE);
            if let Some(c) = clip.animations.continuous.as_ref() {
                s.phase = c.phase;
                s.seams = c.seams;
            }
        }
        Slot::Entrance | Slot::Exit => {
            s.phase = Tick::ZERO;
            s.seams = None;
            // Lines and blocks have no Pace (TX-022).
            if pace_unit(&s).is_none() {
                s.params.pace = None;
            }
            match (clip.content.text(), s.params.pace) {
                (Some(text), Some(_)) => {
                    s = with_paced_duration(&s, text.content.as_str(), rate)?;
                }
                _ => {
                    let other = match slot {
                        Slot::Entrance => clip.animations.exit.as_ref(),
                        _ => clip.animations.entrance.as_ref(),
                    };
                    let taken = other.map_or(Tick::ZERO, |o| o.duration);
                    let room = clip.duration().checked_sub(taken)?;
                    let mut d = rate.quantize(s.duration)?.max(min);
                    if room >= min {
                        d = d.min(rate.round_to_grid(room, Rounding::Floor)?.max(min));
                    }
                    s.duration = d;
                }
            }
        }
    }
    Ok(s)
}
