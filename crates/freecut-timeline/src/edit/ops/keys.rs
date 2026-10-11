//! Volume keyframes (AU-008, with TR-027's rules): adding, changing, moving
//! and removing keys of the clip volume.

use std::sync::Arc;

use super::resolve_clips;
use crate::edit::policy;
use crate::edit::tx::Tx;
use crate::edit::{Change, EditError, Label, OpKind, Plan};
use crate::ids::{ClipId, SeqId};
use crate::model::{Animatable, EmbeddedAudio, GainDb, Interp};
use crate::time::Tick;

/// One change to the volume keyframes of clips (AU-008). Times are each
/// clip's source time (TR-027's anchoring): the app converts the playhead
/// with [`crate::model::Clip::source_time_at`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GainKeyEdit {
    /// Adds a key at `at`, or sets the value of the one there. `None` takes
    /// the volume the clip has there now, so the first key captures the
    /// static volume (TR-027's diamond). Applies to every listed clip with
    /// sound.
    Set { at: Tick, gain: Option<GainDb> },
    /// Sets the interpolation of the segment starting at the key at `at`.
    /// Clips without a key there are skipped.
    Interp { at: Tick, interp: Interp },
    /// Moves the key at `from` toward `to`, clamped strictly between its
    /// neighbours: keys never reorder (AU-008). Clips without a key at
    /// `from` are skipped.
    Move { from: Tick, to: Tick },
    /// Removes the keys at these times. When a clip's last key goes, the
    /// earliest removed key's value becomes its static volume (TR-027).
    /// Clips without a key at any of them are skipped.
    Remove(Vec<Tick>),
    /// Removes every key; the first key's value becomes the static volume.
    /// Clips without keys are left alone.
    Clear,
}

impl GainKeyEdit {
    /// The volume after this edit; `None` when it doesn't apply.
    fn applied_to(&self, gain: &Animatable<GainDb>) -> Option<Animatable<GainDb>> {
        match self {
            GainKeyEdit::Set { at, gain: value } => {
                let value = value.unwrap_or_else(|| gain.value_at(*at));
                Some(gain.with_key(*at, value))
            }
            GainKeyEdit::Interp { at, interp } => gain.with_interp(*at, *interp),
            GainKeyEdit::Move { from, to } => gain.with_key_moved(*from, *to),
            GainKeyEdit::Remove(times) => gain.without_keys(times),
            GainKeyEdit::Clear => gain.is_animated().then(|| gain.cleared()),
        }
    }

    /// The refusal when no listed clip has a key the edit needs.
    fn nothing_found(&self) -> Option<EditError> {
        match self {
            GainKeyEdit::Set { .. } | GainKeyEdit::Clear => None,
            GainKeyEdit::Interp { at, .. } | GainKeyEdit::Move { from: at, .. } => {
                Some(EditError::NoKeyframe { at: *at })
            }
            GainKeyEdit::Remove(times) => Some(EditError::NoKeyframe {
                at: times.first().copied().unwrap_or(Tick::ZERO),
            }),
        }
    }
}

/// Applies `edit` to the volume of every listed clip that plays its own
/// sound, as one undo entry. Clips without sound are skipped, and the edit
/// is refused when none has any; clips on locked tracks refuse it (TL-009).
pub(crate) fn edit_gain_keys(
    mut tx: Tx<'_>,
    seq: SeqId,
    ids: &[ClipId],
    edit: &GainKeyEdit,
) -> Result<Plan, EditError> {
    if ids.is_empty() {
        return Err(EditError::NothingSelected);
    }
    let with_sound: Vec<_> = resolve_clips(&tx, seq, ids)?
        .into_iter()
        .filter_map(|r| r.clip.audio.active().cloned().map(|a| (r, a)))
        .collect();
    if with_sound.is_empty() {
        return Err(EditError::NoClipAudio);
    }
    let targets: Vec<_> = with_sound
        .into_iter()
        .filter_map(|(r, audio)| edit.applied_to(&audio.gain).map(|gain| (r, audio, gain)))
        .collect();
    if targets.is_empty()
        && let Some(err) = edit.nothing_found()
    {
        return Err(err);
    }
    let s = tx.seq(seq)?;
    for (target, ..) in &targets {
        let track = s
            .track_at(target.loc.slot)
            .ok_or(EditError::StaleTrack(target.loc.track))?;
        policy::require_unlocked(track, target.loc.slot)?;
    }
    let mut count: u32 = 0;
    for (target, mut audio, gain) in targets {
        if gain == audio.gain {
            continue;
        }
        audio.gain = gain;
        let mut clip = (*target.clip).clone();
        clip.audio = EmbeddedAudio::Active(audio);
        tx.push(Change::ReplaceClip {
            seq,
            track: target.loc.track,
            before: target.clip,
            after: Arc::new(clip),
        })?;
        count = count.saturating_add(1);
    }
    tx.finish(Label {
        op: OpKind::EditGainKeys,
        count,
    })
}
