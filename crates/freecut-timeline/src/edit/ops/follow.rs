//! *Follow main track* and its item kinds (TL-029).

use std::sync::Arc;

use crate::edit::tx::Tx;
use crate::edit::{Change, EditError, Label, OpKind, Plan};
use crate::ids::SeqId;
use crate::model::{FollowKinds, SequenceFlags, attachment_for, follow_kind};

/// Turns following on or off as one entry: on attaches every item of a
/// checked kind by its current position, off clears every attachment.
/// Nothing moves either way.
pub(crate) fn set_follow_main(tx: Tx<'_>, seq: SeqId, on: bool) -> Result<Plan, EditError> {
    set_follow(tx, seq, OpKind::SetFollowMain, |f| SequenceFlags {
        follow_main: on,
        ..f
    })
}

/// Changes which kinds follow as one entry: newly checked kinds attach by
/// position, unchecked kinds lose their attachments, and the rest keep
/// theirs. Nothing moves.
pub(crate) fn set_follow_kinds(
    tx: Tx<'_>,
    seq: SeqId,
    kinds: FollowKinds,
) -> Result<Plan, EditError> {
    set_follow(tx, seq, OpKind::SetFollowKinds, |f| SequenceFlags {
        follow_kinds: kinds,
        ..f
    })
}

fn set_follow(
    mut tx: Tx<'_>,
    seq: SeqId,
    op: OpKind,
    f: impl FnOnce(SequenceFlags) -> SequenceFlags,
) -> Result<Plan, EditError> {
    let before = tx.seq(seq)?.flags;
    let after = f(before);
    if after == before {
        return tx.finish(Label { op, count: 0 });
    }
    tx.push(Change::SetSeqFlags { seq, before, after })?;
    let mut changes = Vec::new();
    {
        let work = tx.work();
        let s = tx.seq(seq)?;
        for (slot, track) in s.tracks() {
            if slot.is_main() {
                continue;
            }
            for clip in track.clips() {
                let kind = follow_kind(clip, track, work.library());
                let was = before.follow_main && before.follow_kinds.contains(kind);
                let now = after.follow_main && after.follow_kinds.contains(kind);
                let want = match (was, now) {
                    (_, false) => None,
                    (true, true) => clip.attach,
                    (false, true) => attachment_for(s, slot, track, clip, work.library()),
                };
                if want != clip.attach {
                    let mut changed = (**clip).clone();
                    changed.attach = want;
                    changes.push(Change::ReplaceClip {
                        seq,
                        track: track.id,
                        before: Arc::clone(clip),
                        after: Arc::new(changed),
                    });
                }
            }
        }
    }
    for change in changes {
        tx.push(change)?;
    }
    tx.finish(Label { op, count: 1 })
}
