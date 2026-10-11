//! The editor: the only owner of a mutable document.
//!
//! `Document` has no public mutators and a plan's result and changes are
//! read-only, so "every edit goes through the undo stack" holds by
//! construction. Plans are stamped with the editor and revision they were
//! made against; any other plan is refused as stale.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::edit::{
    self, ApplyError, EditCtx, EditError, EditRequest, Label, Limits, Notice, Plan, PlanStamp,
    apply_all,
};
use crate::history::{Coalesce, GestureId, History, HistoryLimits, StateToken};
use crate::ids::{ClipId, IdAlloc, IdCursor};
use crate::model::{Document, InvariantViolation, ProjectSettings, ValidateCtx};

/// Process-wide source of editor identities.
static NEXT_EDITOR: AtomicU64 = AtomicU64::new(1);

fn fresh_editor_id() -> u64 {
    NEXT_EDITOR.fetch_add(1, Ordering::Relaxed)
}

/// Context the app supplies with a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitMeta {
    /// The selection before the edit; undo restores it (TL-015).
    pub selection_before: Vec<ClipId>,
    pub coalesce: Coalesce,
}

impl Default for CommitMeta {
    fn default() -> Self {
        CommitMeta {
            selection_before: Vec::new(),
            coalesce: Coalesce::Never,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Committed {
    /// The plan changed nothing; no history entry was made.
    NoOp,
    Applied {
        notices: Vec<Notice>,
        selection: Option<Vec<ClipId>>,
    },
}

/// What an undo or redo restored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restored {
    pub label: Label,
    /// The selection to show.
    pub selection: Option<Vec<ClipId>>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CommitError {
    /// The plan wasn't made against this editor's current document. Plan
    /// again.
    #[error("the document changed since this edit was planned")]
    Stale,
    #[error("another gesture is in progress")]
    GestureOpen,
    #[error("that gesture isn't in progress")]
    UnknownGesture,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HistoryError {
    #[error("nothing to undo or redo")]
    Empty,
    #[error("finish the current gesture first")]
    GestureOpen,
    /// The recorded changes didn't match the document: a bug. The document is
    /// left untouched and the history is cleared.
    #[error("history diverged from the document: {0}")]
    Diverged(ApplyError),
}

/// Plans edits against a snapshot of an editor's document, for example on
/// a worker thread. Its plans commit only if the editor hasn't changed since
/// the planner was taken.
#[derive(Debug, Clone)]
pub struct Planner {
    doc: Document,
    ids: IdCursor,
    stamp: PlanStamp,
}

impl Planner {
    pub fn document(&self) -> &Document {
        &self.doc
    }

    pub fn plan(&self, ctx: &EditCtx, req: &EditRequest) -> Result<Plan, EditError> {
        let mut plan = edit::plan(&self.doc, ctx, self.ids.clone(), req)?;
        plan.stamp = Some(self.stamp);
        Ok(plan)
    }
}

/// Owns the document, the id allocator and the history.
#[derive(Debug)]
pub struct Editor {
    id: u64,
    doc: Document,
    ids: IdAlloc,
    history: History,
    revision: u64,
}

impl Clone for Editor {
    /// A clone is a separate editor: plans made by one never commit into the
    /// other.
    fn clone(&self) -> Self {
        Editor {
            id: fresh_editor_id(),
            doc: self.doc.clone(),
            ids: self.ids.clone(),
            history: self.history.clone(),
            revision: self.revision,
        }
    }
}

impl Editor {
    /// An editor holding an empty document with default settings.
    pub fn new(limits: HistoryLimits) -> Result<Editor, EditError> {
        Editor::with_settings(ProjectSettings::new_default()?, limits)
    }

    /// An editor holding an empty document with these settings.
    pub fn with_settings(
        settings: ProjectSettings,
        limits: HistoryLimits,
    ) -> Result<Editor, EditError> {
        if !settings.canvas_is_valid() {
            return Err(EditError::InvalidCanvas);
        }
        let mut ids = IdAlloc::default();
        let mut cursor = ids.cursor();
        let seq = cursor.seq()?;
        let main = cursor.track()?;
        ids.advance_to(cursor.high_water());
        Ok(Editor {
            id: fresh_editor_id(),
            doc: Document::new(seq, main, settings),
            ids,
            history: History::new(limits),
            revision: 0,
        })
    }

    /// Opens a loaded document: validates it, then hands it to the editor.
    /// Any non-undoable correction (repair, migration) must happen before
    /// this; everything after is an undoable commit.
    pub fn open(
        doc: Document,
        ids: IdAlloc,
        limits: HistoryLimits,
    ) -> Result<Editor, Vec<InvariantViolation>> {
        let vctx = ValidateCtx {
            limits: Limits::default(),
            id_high_water: None,
        };
        doc.validate(&vctx)?;
        Ok(Editor::from_document(doc, ids, limits))
    }

    /// An editor for a document the caller has already validated. The
    /// allocator is moved past every id in the document (relation ids
    /// included), so a hand-edited file can never cause reuse.
    pub fn from_document(doc: Document, ids: IdAlloc, limits: HistoryLimits) -> Editor {
        let mut ids = ids;
        ids.advance_past(doc.max_id());
        Editor {
            id: fresh_editor_id(),
            doc,
            ids,
            history: History::new(limits),
            revision: 0,
        }
    }

    pub fn document(&self) -> &Document {
        &self.doc
    }

    pub fn ids(&self) -> &IdAlloc {
        &self.ids
    }

    pub fn history(&self) -> &History {
        &self.history
    }

    /// Bumped by every commit, undo and redo.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    fn stamp(&self) -> PlanStamp {
        PlanStamp {
            editor: self.id,
            revision: self.revision,
        }
    }

    pub fn plan(&self, ctx: &EditCtx, req: &EditRequest) -> Result<Plan, EditError> {
        let mut plan = edit::plan(&self.doc, ctx, self.ids.cursor(), req)?;
        plan.stamp = Some(self.stamp());
        Ok(plan)
    }

    /// A planner over a snapshot of the current document, for planning off
    /// the UI thread.
    pub fn planner(&self) -> Planner {
        Planner {
            doc: self.doc.clone(),
            ids: self.ids.cursor(),
            stamp: self.stamp(),
        }
    }

    pub fn commit(&mut self, plan: Plan, meta: CommitMeta) -> Result<Committed, CommitError> {
        if plan.stamp != Some(self.stamp()) {
            return Err(CommitError::Stale);
        }
        match (self.history.gesture_open(), meta.coalesce) {
            (Some(open), Coalesce::Gesture(g)) if g != open => {
                return Err(CommitError::UnknownGesture);
            }
            (Some(_), Coalesce::Gesture(_)) => {}
            (Some(_), _) => return Err(CommitError::GestureOpen),
            (None, Coalesce::Gesture(_)) => return Err(CommitError::UnknownGesture),
            (None, _) => {}
        }
        if plan.is_noop() {
            return Ok(Committed::NoOp);
        }
        self.ids.advance_to(plan.id_high_water);
        let notices = plan.notices().to_vec();
        let (result, label, changes, selection_after) = plan.into_result();
        self.doc = result;
        self.revision += 1;
        // When the edit doesn't decide the selection, it is unchanged; redo
        // restores it either way (TL-015).
        let after = selection_after
            .clone()
            .unwrap_or_else(|| meta.selection_before.clone());
        self.history
            .record(label, changes, meta.selection_before, after, meta.coalesce);
        Ok(Committed::Applied {
            notices,
            selection: selection_after,
        })
    }

    pub fn undo(&mut self) -> Result<Restored, HistoryError> {
        if self.history.gesture_open().is_some() {
            return Err(HistoryError::GestureOpen);
        }
        let entry = self.history.peek_undo().ok_or(HistoryError::Empty)?;
        let restored = Restored {
            label: entry.label,
            selection: Some(entry.selection_before.clone()),
        };
        match apply_all(&self.doc, entry.changes.inverse()) {
            Ok(doc) => {
                self.doc = doc;
                self.revision += 1;
                self.history.shift_to_redo();
                Ok(restored)
            }
            Err(err) => {
                self.history.clear();
                Err(HistoryError::Diverged(err))
            }
        }
    }

    pub fn redo(&mut self) -> Result<Restored, HistoryError> {
        if self.history.gesture_open().is_some() {
            return Err(HistoryError::GestureOpen);
        }
        let entry = self.history.peek_redo().ok_or(HistoryError::Empty)?;
        let restored = Restored {
            label: entry.label,
            selection: Some(entry.selection_after.clone()),
        };
        match apply_all(&self.doc, entry.changes.changes().iter().cloned()) {
            Ok(doc) => {
                self.doc = doc;
                self.revision += 1;
                self.history.shift_to_done();
                Ok(restored)
            }
            Err(err) => {
                self.history.clear();
                Err(HistoryError::Diverged(err))
            }
        }
    }

    /// The current document state, to read together with a save snapshot.
    pub fn generation(&self) -> StateToken {
        self.history.generation()
    }

    /// Records that the state `token` (from [`Editor::generation`]) is on
    /// disk. Edits made while the save ran stay unsaved.
    pub fn mark_saved_at(&mut self, token: StateToken) {
        self.history.mark_saved_at(token);
    }

    /// Records that the current state is on disk (a synchronous save).
    pub fn mark_saved(&mut self) {
        self.history.mark_saved_at(self.history.generation());
    }

    /// Marks the document as having unsaved changes, for example after a
    /// repaired load.
    pub fn mark_unsaved(&mut self) {
        self.history.mark_unsaved();
    }

    pub fn is_dirty(&self) -> bool {
        self.history.is_dirty()
    }

    /// Starts a gesture: commits carrying it merge into one entry. A gesture
    /// still open is ended first.
    pub fn begin_gesture(&mut self) -> GestureId {
        self.history.begin_gesture()
    }

    pub fn end_gesture(&mut self, id: GestureId) {
        self.history.end_gesture(id);
    }

    /// True while a gesture is open: until it ends, only commits carrying it
    /// are accepted, and undo and redo are refused.
    pub fn gesture_open(&self) -> bool {
        self.history.gesture_open().is_some()
    }

    /// Reverts everything gesture `id` committed and leaves the history as
    /// it was before the gesture (pointer capture lost, Esc). Does nothing
    /// unless `id` is the open gesture.
    pub fn cancel_gesture(&mut self, id: GestureId) -> Result<(), HistoryError> {
        if let Some(changes) = self.history.gesture_changes(id) {
            match apply_all(&self.doc, changes.inverse()) {
                Ok(doc) => {
                    self.doc = doc;
                    self.revision += 1;
                }
                Err(err) => {
                    self.history.clear();
                    return Err(HistoryError::Diverged(err));
                }
            }
        }
        self.history.discard_gesture(id);
        Ok(())
    }
}
