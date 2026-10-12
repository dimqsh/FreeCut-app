//! Per-project UI state: what the editor remembers about a project that is
//! not part of the document and never on its undo stack.
//!
//! Today this is AU-012's solo set: "kept in the project's UI state, so it
//! survives reopening, but is not on the undo stack". It is saved in the
//! project file's `ui` object, left out while empty. Changing it does not
//! change the document, so the app decides whether a change alone marks the
//! project unsaved; whatever it holds is written by the next save.

use std::collections::BTreeSet;

use freecut_timeline::ids::TrackId;
use freecut_timeline::model::Document;

/// UI state saved with a project ([`crate::Opened::ui`],
/// [`crate::SaveRequest::ui`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UiState {
    /// Soloed tracks (AU-012). Only tracks of the document that carry sound
    /// are kept: opening and saving drop any other id, so removing a soloed
    /// track clears its solo (AU-012).
    pub solo: BTreeSet<TrackId>,
}

impl UiState {
    /// No UI state.
    pub fn new() -> UiState {
        UiState::default()
    }

    pub fn is_empty(&self) -> bool {
        self.solo.is_empty()
    }

    /// This state without ids that are not sound-carrying tracks of `doc`.
    pub fn retained_for(&self, doc: &Document) -> UiState {
        UiState {
            solo: self
                .solo
                .iter()
                .copied()
                .filter(|&id| carries_sound(doc, id))
                .collect(),
        }
    }
}

/// Whether `track` is a track of `doc` whose kind carries sound.
fn carries_sound(doc: &Document, track: TrackId) -> bool {
    doc.sequences()
        .flat_map(|seq| seq.visual_tracks().iter().chain(seq.audio_tracks()))
        .any(|t| t.id == track && t.kind.carries_sound())
}
