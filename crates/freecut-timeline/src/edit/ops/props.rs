//! Clip properties: the clip transform (TR-001, TR-003, TR-015), and the
//! shared rule of every property edit (text edits are in [`super::text`]).
//!
//! Each edit sets one value on every eligible listed clip; ineligible clips
//! are skipped, and the edit is refused when none is eligible. Clips on
//! locked tracks refuse the whole edit. Values are clamped into their
//! ranges, and clips already holding the value are left alone, so an edit
//! that changes nothing records nothing.

use std::sync::Arc;

use super::{Resolved, resolve_clips};
use crate::edit::policy;
use crate::edit::tx::Tx;
use crate::edit::{Change, EditError, Label, OpKind, Plan};
use crate::ids::{ClipId, SeqId};
use crate::model::{Angle, CanvasPos, Clip, PerMille, Scale, Transform};

/// One transform value to put on visual clips (TR-001, TR-003, TR-015).
/// Values are clamped to their TR-001 ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransformEdit {
    Position(CanvasPos),
    Scale(Scale),
    Rotation(Angle),
    FlipH(bool),
    FlipV(bool),
    Opacity(PerMille),
    /// The Transform section's reset (TR-001): position, scale, rotation
    /// and flips back to their defaults. Opacity lives in the Compositing
    /// section (TR-015) and is kept.
    Reset,
}

impl TransformEdit {
    fn applied_to(self, t: Transform) -> Transform {
        let out = match self {
            TransformEdit::Position(position) => Transform { position, ..t },
            TransformEdit::Scale(scale) => Transform { scale, ..t },
            TransformEdit::Rotation(rotation) => Transform { rotation, ..t },
            TransformEdit::FlipH(flip_h) => Transform { flip_h, ..t },
            TransformEdit::FlipV(flip_v) => Transform { flip_v, ..t },
            TransformEdit::Opacity(opacity) => Transform { opacity, ..t },
            TransformEdit::Reset => Transform {
                opacity: t.opacity,
                ..Transform::IDENTITY
            },
        };
        out.clamped()
    }
}

pub(crate) fn set_transform(
    tx: Tx<'_>,
    seq: SeqId,
    clips: &[ClipId],
    edit: TransformEdit,
) -> Result<Plan, EditError> {
    update_clips(
        tx,
        seq,
        clips,
        OpKind::SetTransform,
        EditError::NoVisualClip,
        |clip| {
            if !clip.content.is_visual() {
                return None;
            }
            let mut out = clip.clone();
            out.transform = edit.applied_to(clip.transform);
            Some(out)
        },
    )
}

/// Replaces every listed clip for which `f` gives a new version. Clips `f`
/// skips (`None`) are left alone; when it skips all of them the edit is
/// refused with `none`. Any eligible clip on a locked track refuses the
/// edit. Unchanged clips record nothing.
pub(super) fn update_clips(
    tx: Tx<'_>,
    seq: SeqId,
    ids: &[ClipId],
    op: OpKind,
    none: EditError,
    f: impl Fn(&Clip) -> Option<Clip>,
) -> Result<Plan, EditError> {
    try_update_clips(tx, seq, ids, op, none, |clip| Ok(f(clip)))
}

/// [`update_clips`] with a fallible `f`: its first error refuses the edit.
pub(super) fn try_update_clips(
    mut tx: Tx<'_>,
    seq: SeqId,
    ids: &[ClipId],
    op: OpKind,
    none: EditError,
    f: impl Fn(&Clip) -> Result<Option<Clip>, EditError>,
) -> Result<Plan, EditError> {
    if ids.is_empty() {
        return Err(EditError::NothingSelected);
    }
    let mut targets: Vec<(Resolved, Clip)> = Vec::new();
    for r in resolve_clips(&tx, seq, ids)? {
        if let Some(after) = f(&r.clip)? {
            targets.push((r, after));
        }
    }
    if targets.is_empty() {
        return Err(none);
    }
    let s = tx.seq(seq)?;
    for (target, _) in &targets {
        let track = s
            .track_at(target.loc.slot)
            .ok_or(EditError::StaleTrack(target.loc.track))?;
        policy::require_unlocked(track, target.loc.slot)?;
    }
    let mut count: u32 = 0;
    for (target, after) in targets {
        if after == *target.clip {
            continue;
        }
        tx.push(Change::ReplaceClip {
            seq,
            track: target.loc.track,
            before: target.clip,
            after: Arc::new(after),
        })?;
        count = count.saturating_add(1);
    }
    tx.finish(Label { op, count })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_edits_clamp_and_reset_keeps_opacity() {
        let mut t = Transform::IDENTITY;
        t = TransformEdit::Scale(Scale::from_basis_points(0)).applied_to(t);
        assert_eq!(t.scale, Scale::MIN);
        t = TransformEdit::Rotation(Angle::from_degrees(-4000)).applied_to(t);
        assert_eq!(t.rotation, Angle::from_degrees(-3600));
        t = TransformEdit::Position(CanvasPos::new(5_000_000, 1)).applied_to(t);
        assert_eq!(t.position, CanvasPos::new(4_000_000, 1));
        t = TransformEdit::Opacity(PerMille::new(1200)).applied_to(t);
        assert_eq!(t.opacity, PerMille::ONE);
        t = TransformEdit::Opacity(PerMille::new(250)).applied_to(t);
        t = TransformEdit::FlipV(true).applied_to(t);
        let reset = TransformEdit::Reset.applied_to(t);
        assert_eq!(
            reset,
            Transform {
                opacity: PerMille::new(250),
                ..Transform::IDENTITY
            }
        );
    }
}
