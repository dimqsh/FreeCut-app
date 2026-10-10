//! Stable entity ids (TL-006).
//!
//! Ids come from one monotonic, document-wide counter. Undo never lowers it,
//! so an id is never reused, and undoing a delete restores the original id.

use core::num::NonZeroU64;

use crate::edit::EditError;

macro_rules! entity_id {
    ($($(#[$meta:meta])* $name:ident),* $(,)?) => {$(
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(NonZeroU64);

        impl $name {
            pub const fn get(self) -> u64 {
                self.0.get()
            }

            pub const fn nonzero(self) -> NonZeroU64 {
                self.0
            }

            /// For loaders and tests; editing code allocates through [`IdCursor`].
            pub const fn from_raw(raw: NonZeroU64) -> Self {
                Self(raw)
            }
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{}#{}", stringify!($name), self.0)
            }
        }
    )*};
}

entity_id!(
    /// A sequence (the root timeline or a nested compound sequence).
    SeqId,
    /// A track.
    TrackId,
    /// A clip.
    ClipId,
    /// A clip group (TL-039).
    GroupId,
    /// A link between clips that belong together (TL-030).
    LinkId,
    /// A media library item (PM-030).
    MediaId,
    /// A timeline or clip marker (TL-037).
    MarkerId,
);

/// The largest id ever handed out: 2^53, so ids survive any JSON reader that
/// parses numbers as `f64`.
pub const MAX_ID: u64 = 1 << 53;

/// The persisted, monotonic id counter. Lives in the editor, not the document,
/// so document equality ignores it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdAlloc {
    next: NonZeroU64,
}

impl Default for IdAlloc {
    fn default() -> Self {
        IdAlloc {
            next: NonZeroU64::MIN,
        }
    }
}

impl IdAlloc {
    /// Resumes a persisted counter.
    pub fn starting_at(next: NonZeroU64) -> IdAlloc {
        IdAlloc { next }
    }

    /// The next id that will be handed out.
    pub const fn next(&self) -> NonZeroU64 {
        self.next
    }

    /// A planning cursor starting at the current position. Planning burns no
    /// ids until the plan is committed.
    pub const fn cursor(&self) -> IdCursor {
        IdCursor { next: self.next }
    }

    /// Moves the counter forward to `high_water`; never backward.
    pub fn advance_to(&mut self, high_water: NonZeroU64) {
        self.next = self.next.max(high_water);
    }

    /// Moves the counter past `max_used`. If that isn't possible, the
    /// counter is pinned at exhaustion: allocation then fails rather than
    /// ever reusing an id.
    pub fn advance_past(&mut self, max_used: u64) {
        let next = max_used
            .checked_add(1)
            .and_then(NonZeroU64::new)
            .unwrap_or(NonZeroU64::MAX);
        self.advance_to(next);
    }
}

/// Allocates ids while a plan is built.
#[derive(Debug, Clone)]
pub struct IdCursor {
    next: NonZeroU64,
}

impl IdCursor {
    pub const fn high_water(&self) -> NonZeroU64 {
        self.next
    }

    fn bump(&mut self) -> Result<NonZeroU64, EditError> {
        let id = self.next;
        if id.get() > MAX_ID {
            return Err(EditError::IdSpaceExhausted);
        }
        self.next = id.checked_add(1).ok_or(EditError::IdSpaceExhausted)?;
        Ok(id)
    }

    pub fn seq(&mut self) -> Result<SeqId, EditError> {
        self.bump().map(SeqId)
    }

    pub fn track(&mut self) -> Result<TrackId, EditError> {
        self.bump().map(TrackId)
    }

    pub fn clip(&mut self) -> Result<ClipId, EditError> {
        self.bump().map(ClipId)
    }

    pub fn group(&mut self) -> Result<GroupId, EditError> {
        self.bump().map(GroupId)
    }

    pub fn link(&mut self) -> Result<LinkId, EditError> {
        self.bump().map(LinkId)
    }

    pub fn media(&mut self) -> Result<MediaId, EditError> {
        self.bump().map(MediaId)
    }

    pub fn marker(&mut self) -> Result<MarkerId, EditError> {
        self.bump().map(MarkerId)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_allocates_sequentially_and_alloc_only_moves_forward() {
        let mut alloc = IdAlloc::default();
        let mut cursor = alloc.cursor();
        let a = cursor.clip().unwrap();
        let b = cursor.track().unwrap();
        assert_eq!((a.get(), b.get()), (1, 2));
        alloc.advance_to(cursor.high_water());
        assert_eq!(alloc.next().get(), 3);
        alloc.advance_to(NonZeroU64::MIN);
        assert_eq!(alloc.next().get(), 3, "never moves backward");
    }

    #[test]
    fn ids_stop_at_two_to_the_53rd() {
        let alloc = IdAlloc::starting_at(NonZeroU64::new(MAX_ID).unwrap());
        let mut cursor = alloc.cursor();
        assert_eq!(cursor.clip().unwrap().get(), MAX_ID);
        assert_eq!(cursor.clip(), Err(EditError::IdSpaceExhausted));
    }

    #[test]
    fn advancing_past_the_largest_id_pins_the_allocator() {
        let mut alloc = IdAlloc::default();
        alloc.advance_past(u64::MAX);
        assert_eq!(alloc.cursor().clip(), Err(EditError::IdSpaceExhausted));
        let mut alloc = IdAlloc::default();
        alloc.advance_past(41);
        assert_eq!(alloc.next().get(), 42);
    }

    #[test]
    fn exhausted_id_space_is_an_error_not_a_panic() {
        let alloc = IdAlloc::starting_at(NonZeroU64::MAX);
        let mut cursor = alloc.cursor();
        assert_eq!(cursor.clip(), Err(EditError::IdSpaceExhausted));
    }
}
