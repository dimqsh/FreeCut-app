//! Unknown keys from the project file, kept so they survive open → save.
//!
//! A file written by a newer or development build can carry keys this build
//! doesn't know. Dropping them on save would silently destroy someone's data
//! (PM-021), so the reader collects every unknown key, at every object level,
//! into an [`Extras`] value returned next to the document. The timeline model
//! never sees them.
//!
//! Each set of unknown keys is filed under the entity that owns the object
//! ([`Owner`]) and the object's path inside that entity (`""` for the entity
//! object itself, `"range"`, `"content.source"`, `"info.video[0]"`, …). On
//! save the writer re-attaches them to the same place, but only for entities
//! that still exist: a deleted clip takes its unknown keys with it, and new
//! entities (the right half of a split, a fresh import) start without any.
//!
//! Objects inside arrays that are not entities (media streams) are filed by
//! position, `info.video[1]`: after a relink that changes the streams, their
//! unknown keys stay with whichever stream now has that position. Volume
//! keyframes are filed by their time instead, `audio.gain_keys@<ticks>`, so
//! adding or removing an earlier keyframe doesn't hand their unknown keys to
//! a neighbour; a keyframe that is moved or deleted drops them.
//!
//! Objects with variants (a clip's `content` and `audio`, an item's
//! `source`) are filed by path, whatever their variant: when detaching turns
//! a clip's `"active"` sound into `"detached"`, the unknown keys of the old
//! object are written into the new one, except those the new variant writes
//! itself (a `link` from an older `"active"` object is dropped once the
//! object is `"detached"` with a link of its own).

use std::collections::BTreeMap;

use freecut_timeline::ids::{ClipId, MarkerId, MediaId, SeqId, TrackId};
use serde_json::Value;

/// Unknown keys of one JSON object, in key order.
pub type Fields = BTreeMap<String, Value>;

/// The entity an object with unknown keys belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Owner {
    /// The file's top-level object (the envelope).
    Root,
    /// The `document` object.
    Document,
    /// The project settings and their sub-objects.
    Settings,
    Media(MediaId),
    Sequence(SeqId),
    Track(TrackId),
    Clip(ClipId),
    /// A timeline or clip marker (format 7). Filed by its id, so a clip
    /// marker keeps its unknown keys when a split hands it to the right
    /// piece.
    Marker(MarkerId),
    /// The saved export settings (`export`) and their sub-objects.
    Export,
    /// The project's UI state (`ui`).
    Ui,
}

/// Unknown keys from an opened file, keyed by owner and object path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Extras {
    by_owner: BTreeMap<Owner, BTreeMap<String, Fields>>,
}

impl Extras {
    /// No unknown keys.
    pub fn new() -> Extras {
        Extras::default()
    }

    pub fn is_empty(&self) -> bool {
        self.by_owner.is_empty()
    }

    /// Number of objects that carry unknown keys.
    pub fn len(&self) -> usize {
        self.by_owner.values().map(BTreeMap::len).sum()
    }

    /// The unknown keys of the object at `at` inside `owner` (`""` for the
    /// owner's own object).
    pub fn get(&self, owner: Owner, at: &str) -> Option<&Fields> {
        self.by_owner.get(&owner)?.get(at)
    }

    /// Every object with unknown keys, by owner then path.
    pub fn iter(&self) -> impl Iterator<Item = (Owner, &str, &Fields)> {
        self.by_owner.iter().flat_map(|(owner, objects)| {
            objects
                .iter()
                .map(move |(at, fields)| (*owner, at.as_str(), fields))
        })
    }

    /// Files `fields` under `owner`/`at` (reader side). Empty sets are not
    /// stored.
    pub(crate) fn put(&mut self, owner: Owner, at: impl Into<String>, fields: Fields) {
        if !fields.is_empty() {
            self.by_owner
                .entry(owner)
                .or_default()
                .insert(at.into(), fields);
        }
    }

    /// Whether `owner` has any unknown keys (writer fast path).
    pub(crate) fn has(&self, owner: Owner) -> bool {
        self.by_owner.contains_key(&owner)
    }

    /// A copy of the unknown keys to write back at `owner`/`at` (writer side).
    pub(crate) fn fields(&self, owner: Owner, at: &str) -> Fields {
        self.get(owner, at).cloned().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::num::NonZeroU64;

    fn clip(n: u64) -> Owner {
        Owner::Clip(ClipId::from_raw(NonZeroU64::new(n).unwrap()))
    }

    #[test]
    fn empty_sets_are_not_stored_and_lookups_are_by_owner_and_path() {
        let mut extras = Extras::new();
        extras.put(clip(1), "range", Fields::new());
        assert!(extras.is_empty());

        let mut fields = Fields::new();
        fields.insert("x".into(), Value::from(1));
        extras.put(clip(1), "range", fields.clone());
        extras.put(Owner::Root, "", fields.clone());
        assert_eq!(extras.len(), 2);
        assert!(extras.has(clip(1)));
        assert!(!extras.has(clip(2)));
        assert_eq!(extras.get(clip(1), "range"), Some(&fields));
        assert_eq!(extras.get(clip(1), ""), None);
        assert_eq!(extras.fields(clip(2), "range"), Fields::new());
        let owners: Vec<_> = extras
            .iter()
            .map(|(o, at, _)| (o, at.to_string()))
            .collect();
        assert_eq!(
            owners,
            vec![(Owner::Root, String::new()), (clip(1), "range".to_string())]
        );
    }
}
