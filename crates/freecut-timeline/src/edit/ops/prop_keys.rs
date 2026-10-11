//! Keyframes of transform and text properties (TR-027 to TR-031, TX-021):
//! add, remove, move, set the interpolation, and the card-wide commands
//! **Key this card** and **Remove this card's keys**.
//!
//! Times are sequence times (the playhead), converted on each clip to its
//! content time ([`Clip::content_time_at`]), so one request serves a
//! multi-clip selection (TR-028). Volume keys keep their own request,
//! [`super::GainKeyEdit`], in source time.

use std::sync::Arc;

use super::props::try_update_clips;
use crate::edit::tx::Tx;
use crate::edit::{EditError, OpKind, Plan};
use crate::ids::{ClipId, SeqId};
use crate::model::{Animatable, Clip, Interp, PropGroup, PropPath, PropValue};
use crate::time::Tick;

/// One change to the keyframes of clips (TR-028, TR-029, TR-031, TX-021).
/// `at`, `from` and `to` are sequence times.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyEdit {
    /// Adds a key at the playhead `at`, or sets the value of the key there.
    /// `None` takes the value the property has there now, so the first key
    /// captures the static value (TR-027). Values are clamped into the
    /// property's range. Clips not under the playhead are skipped (TR-028).
    Add { at: Tick, value: Option<PropValue> },
    /// Removes the key at `at`. When a clip's last key goes, its value
    /// becomes the static value (TR-027). Clips without a key there are
    /// skipped.
    Remove { at: Tick },
    /// Moves the key at `from` toward `to`, clamped strictly between its
    /// neighbours (TR-029). Clips without a key at `from` are skipped.
    Move { from: Tick, to: Tick },
    /// Sets the interpolation of the segment starting at the key at `at`
    /// (TR-031). Clips without a key there are skipped.
    SetInterp { at: Tick, interp: Interp },
    /// **Key this card** (TX-021) or TR-028's "Add keyframe for all
    /// transform properties": a key at the playhead, with its current
    /// value, on every property of `card` the clip has and that has no key
    /// there yet. Clips not under the playhead are skipped.
    KeyCard { card: PropGroup, at: Tick },
    /// **Remove this card's keys here** (`at`: the playhead; clips without
    /// such a key are skipped), or every key of the card (`None`; clips
    /// without any are left alone); a property left without keys keeps the
    /// value of its earliest removed key (TR-027).
    ClearCard { card: PropGroup, at: Option<Tick> },
}

impl KeyEdit {
    fn card(self) -> Option<PropGroup> {
        match self {
            KeyEdit::KeyCard { card, .. } | KeyEdit::ClearCard { card, .. } => Some(card),
            _ => None,
        }
    }
}

/// Applies `edit` to `prop` (ignored by the card edits) on every listed
/// clip that has the property, as one undo entry. Refused when the
/// property takes no keys or the value has the wrong kind, when no listed
/// clip has it, and when the edit applies to none of them (no clip under
/// the playhead, no key at the time); clips on locked tracks refuse it.
pub(crate) fn edit_keys(
    tx: Tx<'_>,
    seq: SeqId,
    ids: &[ClipId],
    prop: PropPath,
    edit: KeyEdit,
) -> Result<Plan, EditError> {
    let group = edit.card();
    if group.is_none() {
        let kind = prop.kind().ok_or(EditError::NotAnimatable)?;
        if let KeyEdit::Add { value: Some(v), .. } = edit
            && v.kind() != kind
        {
            return Err(EditError::WrongValueKind);
        }
    }
    let has = |clip: &Clip| match group {
        Some(g) => {
            !g.props(clip).is_empty() || clip.keys.tracks().iter().any(|t| g.contains(t.prop()))
        }
        None => prop.get(clip).is_some(),
    };
    let transform = matches!(group, Some(PropGroup::Transform))
        || (group.is_none() && matches!(prop, PropPath::Transform(_)));
    let color = matches!(group, Some(PropGroup::Color(_)))
        || (group.is_none() && matches!(prop, PropPath::Color(_) | PropPath::ColorPuck(_)));
    let none = if transform {
        EditError::NoVisualClip
    } else if color {
        EditError::NoColorClip
    } else {
        EditError::NoTextClip
    };
    // Which refusal applies when every clip that has the property skips.
    let skipped = match edit {
        KeyEdit::Add { .. } | KeyEdit::KeyCard { .. } => EditError::NotUnderPlayhead,
        KeyEdit::Remove { at } | KeyEdit::SetInterp { at, .. } | KeyEdit::Move { from: at, .. } => {
            EditError::NoKey { at }
        }
        KeyEdit::ClearCard { at, .. } => EditError::NoKey {
            at: at.unwrap_or(Tick::ZERO),
        },
    };
    let resolved_any = std::cell::Cell::new(false);
    let plan = try_update_clips(tx, seq, ids, OpKind::EditKeys, none.clone(), |clip| {
        if !has(clip) {
            return Ok(None);
        }
        resolved_any.set(true);
        Ok(apply(clip, prop, edit))
    });
    match plan {
        Err(e) if e == none && resolved_any.get() => Err(skipped),
        other => other,
    }
}

/// The clip after `edit`, or `None` when the edit doesn't apply to it.
fn apply(clip: &Clip, prop: PropPath, edit: KeyEdit) -> Option<Clip> {
    let content = |at: Tick| clip.content_time_at(at);
    match edit {
        KeyEdit::Add { at, value } => {
            if !clip.range.contains(at) {
                return None;
            }
            let ct = content(at)?;
            let a = clip.animatable(prop)?;
            let v = match value {
                Some(v) => prop.clamp_value(clip, v)?,
                None => a.value_at(ct),
            };
            Some(written(clip, prop, &a.with_key(ct, v)))
        }
        KeyEdit::Remove { at } => {
            let ct = content(at)?;
            edit_track(clip, prop, |a| a.without_keys(&[ct]))
        }
        KeyEdit::Move { from, to } => {
            let (from, to) = (content(from)?, content(to)?);
            edit_track(clip, prop, |a| a.with_key_moved(from, to))
        }
        KeyEdit::SetInterp { at, interp } => {
            let ct = content(at)?;
            edit_track(clip, prop, |a| a.with_interp(ct, interp))
        }
        KeyEdit::KeyCard { card, at } => {
            if !clip.range.contains(at) {
                return None;
            }
            let ct = content(at)?;
            let mut out = clip.clone();
            for p in card.props(clip) {
                let a = out.animatable(p)?;
                if a.key_at(ct).is_none() {
                    let v = a.value_at(ct);
                    out = written(&out, p, &a.with_key(ct, v));
                }
            }
            Some(out)
        }
        KeyEdit::ClearCard { card, at } => {
            let ct = match at {
                Some(at) => Some(content(at)?),
                None => None,
            };
            let mut out = clip.clone();
            let mut changed = false;
            let keyed: Vec<PropPath> = clip
                .keys
                .tracks()
                .iter()
                .map(|t| t.prop())
                .filter(|p| card.contains(*p))
                .collect();
            for p in keyed {
                let next = edit_track(&out, p, |a| match ct {
                    Some(ct) => a.without_keys(&[ct]),
                    None => Some(a.cleared()),
                });
                if let Some(next) = next {
                    out = next;
                    changed = true;
                }
            }
            // Clearing every key of a card that has none changes nothing
            // and refuses nothing; clearing at a time needs a key there.
            (changed || ct.is_none()).then_some(out)
        }
    }
}

/// Applies `f` to the keys of `prop` on `clip`. A property the clip lacks
/// right now (a keyed solid colour under a gradient) is edited against its
/// first key's value as the static value; `None` when `f` doesn't apply.
fn edit_track(
    clip: &Clip,
    prop: PropPath,
    f: impl FnOnce(&Animatable<PropValue>) -> Option<Animatable<PropValue>>,
) -> Option<Clip> {
    let a = match clip.animatable(prop) {
        Some(a) => a,
        None => {
            let keys = clip.keys.tracks().iter().find(|t| t.prop() == prop)?;
            let first = keys.keys().first()?.value;
            Animatable::from_parts(first, keys.keys().to_vec()).ok()?
        }
    };
    Some(written(clip, prop, &f(&a)?))
}

/// `clip` with `prop`'s keys from `a`, and, once no key is left, `a`'s
/// static value written back (TR-027).
fn written(clip: &Clip, prop: PropPath, a: &Animatable<PropValue>) -> Clip {
    let mut out = clip.clone();
    out.keys = out.keys.with_track(prop, Arc::clone(a.shared_keys()));
    if !a.is_animated() {
        // A property the clip lacks keeps nothing (`set` refuses it).
        let _ = prop.set(&mut out, *a.static_value());
    }
    out
}
