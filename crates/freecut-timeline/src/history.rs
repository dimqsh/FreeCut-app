//! The linear undo history (TL-043).
//!
//! Every entry is one user action. Continuous gestures merge into one entry;
//! keyboard repeats within a second merge too. The history also tracks which
//! document state was last saved, for the dirty flag.

use std::collections::VecDeque;

use crate::edit::{ChangeSet, Label, OpKind};
use crate::ids::ClipId;

/// Identifies one continuous gesture (pointer down to pointer up). Every
/// commit made during it merges into one history entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GestureId(u64);

/// What a repeated command acts on, for merging keyboard repeats (nudges,
/// volume steps) made within a second of each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CoalesceKey {
    pub op: OpKind,
    /// A stable hash of the target ids or property path.
    pub target: u64,
}

/// How a commit may merge with the previous history entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coalesce {
    Never,
    Gesture(GestureId),
    Repeat { key: CoalesceKey, now_ms: u64 },
}

/// Identifies a document state in the history, for saving in the
/// background: read it when taking the save snapshot, and hand it back to
/// [`crate::Editor::mark_saved_at`] when the write has finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StateToken(u64);

const REPEAT_WINDOW_MS: u64 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryLimits {
    pub max_entries: usize,
    /// Memory budget for change data. The oldest entries go first; the
    /// newest entry is always kept, even if it alone is larger.
    pub max_bytes: usize,
}

impl Default for HistoryLimits {
    fn default() -> Self {
        HistoryLimits {
            max_entries: 200,
            max_bytes: 512 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Entry {
    id: u64,
    pub(crate) label: Label,
    pub(crate) changes: ChangeSet,
    pub(crate) selection_before: Vec<ClipId>,
    pub(crate) selection_after: Vec<ClipId>,
    state_after: u64,
    repeat: Option<(CoalesceKey, u64)>,
    sealed: bool,
    bytes: usize,
}

/// What [`History::record`] did with a commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Recorded {
    New,
    Merged,
    /// The merged changes cancelled out; the entry is gone.
    Cancelled,
}

/// The gesture in progress.
#[derive(Debug, Clone)]
struct OpenGesture {
    id: GestureId,
    /// The entry this gesture is building, while it exists.
    entry: Option<u64>,
    /// Bookkeeping a normal commit does at once is deferred to the end of
    /// the gesture, so a cancel can leave no trace.
    pending_cleanup: bool,
    saved_during: bool,
}

/// Done and undone entries plus the saved-position bookkeeping.
#[derive(Debug, Clone)]
pub struct History {
    done: VecDeque<Entry>,
    undone: Vec<Entry>,
    /// The document state below `done[0]`.
    base_state: u64,
    saved: Option<u64>,
    next_state: u64,
    next_entry: u64,
    next_gesture: u64,
    limits: HistoryLimits,
    bytes: usize,
    gesture: Option<OpenGesture>,
    /// The entry committed last, while repeats may still merge into it.
    last_commit: Option<u64>,
}

impl History {
    pub fn new(limits: HistoryLimits) -> History {
        History {
            done: VecDeque::new(),
            undone: Vec::new(),
            base_state: 0,
            // A fresh document counts as saved; it has no unsaved changes.
            saved: Some(0),
            next_state: 1,
            next_entry: 0,
            next_gesture: 0,
            limits,
            bytes: 0,
            gesture: None,
            last_commit: None,
        }
    }

    fn current_state(&self) -> u64 {
        self.done.back().map_or(self.base_state, |e| e.state_after)
    }

    fn fresh_state(&mut self) -> u64 {
        let s = self.next_state;
        self.next_state += 1;
        s
    }

    fn reachable(&self, state: u64) -> bool {
        state == self.base_state
            || self.done.iter().any(|e| e.state_after == state)
            || self.undone.iter().any(|e| e.state_after == state)
    }

    pub fn is_dirty(&self) -> bool {
        self.saved != Some(self.current_state())
    }

    /// The current document state.
    pub fn generation(&self) -> StateToken {
        StateToken(self.current_state())
    }

    /// Records that the document state `token` is on disk. If that state is
    /// no longer reachable by undo or redo, nothing is clean any more.
    pub fn mark_saved_at(&mut self, token: StateToken) {
        self.saved = self.reachable(token.0).then_some(token.0);
        if let Some(g) = &mut self.gesture {
            g.saved_during = true;
        }
        let open_entry = self.gesture.as_ref().and_then(|g| g.entry);
        if let Some(top) = self.done.back_mut()
            && top.state_after == token.0
            && Some(top.id) != open_entry
        {
            // Merging into the saved entry would make the save point
            // unreachable. A live gesture keeps merging; its new state is
            // then simply unsaved.
            top.sealed = true;
        }
        self.last_commit = None;
    }

    /// Marks the document as having unsaved changes (for example after a
    /// repaired load or a crash recovery).
    pub fn mark_unsaved(&mut self) {
        self.saved = None;
    }

    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    pub fn undo_label(&self) -> Option<Label> {
        self.done.back().map(|e| e.label)
    }

    pub fn redo_label(&self) -> Option<Label> {
        self.undone.last().map(|e| e.label)
    }

    /// Number of entries that can be undone.
    pub fn len(&self) -> usize {
        self.done.len()
    }

    pub fn is_empty(&self) -> bool {
        self.done.is_empty()
    }

    /// Approximate memory held by change data, done and undone.
    pub fn heap_bytes(&self) -> usize {
        self.bytes
    }

    pub(crate) fn gesture_open(&self) -> Option<GestureId> {
        self.gesture.as_ref().map(|g| g.id)
    }

    pub(crate) fn begin_gesture(&mut self) -> GestureId {
        if let Some(open) = self.gesture_open() {
            self.end_gesture(open);
        }
        let id = GestureId(self.next_gesture);
        self.next_gesture += 1;
        self.gesture = Some(OpenGesture {
            id,
            entry: None,
            pending_cleanup: false,
            saved_during: false,
        });
        id
    }

    pub(crate) fn end_gesture(&mut self, id: GestureId) {
        let Some(g) = self.gesture.take_if(|g| g.id == id) else {
            return;
        };
        if g.pending_cleanup {
            self.discard_redo_branch();
            self.evict();
        }
        if let Some(top) = self.done.back_mut()
            && Some(top.id) == g.entry
        {
            top.sealed = true;
        }
    }

    /// The changes the open gesture `id` has committed, newest entry first.
    pub(crate) fn gesture_changes(&self, id: GestureId) -> Option<&ChangeSet> {
        let g = self.gesture.as_ref().filter(|g| g.id == id)?;
        self.done
            .back()
            .filter(|top| Some(top.id) == g.entry)
            .map(|top| &top.changes)
    }

    /// Forgets the open gesture `id` after the editor has reverted it. The
    /// history is left exactly as it was before the gesture began, except
    /// that a save made during the gesture of a now-unreachable state leaves
    /// the document dirty.
    pub(crate) fn discard_gesture(&mut self, id: GestureId) {
        let Some(g) = self.gesture.take_if(|g| g.id == id) else {
            return;
        };
        if let Some(top) = self.done.back()
            && Some(top.id) == g.entry
        {
            let bytes = top.bytes;
            self.done.pop_back();
            self.bytes = self.bytes.saturating_sub(bytes);
        }
        if g.saved_during && self.saved.is_some_and(|s| !self.reachable(s)) {
            self.saved = None;
        }
        self.last_commit = None;
    }

    /// Records a committed edit.
    pub(crate) fn record(
        &mut self,
        label: Label,
        changes: ChangeSet,
        selection_before: Vec<ClipId>,
        selection_after: Vec<ClipId>,
        how: Coalesce,
    ) -> Recorded {
        let merge_into = match how {
            Coalesce::Gesture(id) => self
                .gesture
                .as_ref()
                .filter(|g| g.id == id)
                .and_then(|g| g.entry),
            Coalesce::Repeat { key, now_ms } => self.last_commit.filter(|_| {
                self.done.back().is_some_and(|top| {
                    !top.sealed
                        && top.repeat.is_some_and(|(k, at)| {
                            k == key && now_ms.saturating_sub(at) <= REPEAT_WINDOW_MS
                        })
                })
            }),
            Coalesce::Never => None,
        };
        let state = self.fresh_state();
        if let Some(entry_id) = merge_into
            && let Some(top) = self.done.back_mut()
            && top.id == entry_id
        {
            top.changes.extend(changes);
            top.changes.compact();
            top.selection_after = selection_after;
            top.state_after = state;
            if let Coalesce::Repeat { key, now_ms } = how {
                top.repeat = Some((key, now_ms));
            }
            let old = top.bytes;
            top.bytes = top.changes.heap_bytes();
            self.bytes = self.bytes.saturating_sub(old) + top.bytes;
            if top.changes.is_empty() {
                // The merged edits cancelled out: no entry at all.
                let bytes = top.bytes;
                self.done.pop_back();
                self.bytes = self.bytes.saturating_sub(bytes);
                self.last_commit = None;
                if let Some(g) = &mut self.gesture {
                    g.entry = None;
                }
                return Recorded::Cancelled;
            }
            return Recorded::Merged;
        }

        let id = self.next_entry;
        self.next_entry += 1;
        let bytes = changes.heap_bytes();
        let entry = Entry {
            id,
            label,
            changes,
            selection_before,
            selection_after,
            state_after: state,
            repeat: match how {
                Coalesce::Repeat { key, now_ms } => Some((key, now_ms)),
                _ => None,
            },
            sealed: false,
            bytes,
        };
        let gesture = match how {
            Coalesce::Gesture(gid) => self.gesture.as_mut().filter(|g| g.id == gid),
            _ => None,
        };
        if let Some(g) = gesture {
            // Defer the redo-branch discard and eviction to end_gesture, so
            // a cancel leaves no trace.
            g.entry = Some(id);
            g.pending_cleanup = true;
            self.done.push_back(entry);
            self.bytes += bytes;
            self.last_commit = None;
        } else {
            self.discard_redo_branch();
            self.done.push_back(entry);
            self.bytes += bytes;
            self.evict();
            self.last_commit = Some(id);
        }
        Recorded::New
    }

    fn discard_redo_branch(&mut self) {
        if self
            .saved
            .is_some_and(|s| self.undone.iter().any(|e| e.state_after == s))
        {
            self.saved = None;
        }
        let freed: usize = self.undone.iter().map(|e| e.bytes).sum();
        self.bytes = self.bytes.saturating_sub(freed);
        self.undone.clear();
    }

    /// Drops the oldest entries while over either limit, always keeping the
    /// newest.
    fn evict(&mut self) {
        while self.done.len() > 1
            && (self.done.len() > self.limits.max_entries.max(1)
                || self.bytes > self.limits.max_bytes)
        {
            if let Some(evicted) = self.done.pop_front() {
                self.base_state = evicted.state_after;
                self.bytes = self.bytes.saturating_sub(evicted.bytes);
            }
        }
    }

    pub(crate) fn peek_undo(&self) -> Option<&Entry> {
        self.done.back()
    }

    pub(crate) fn peek_redo(&self) -> Option<&Entry> {
        self.undone.last()
    }

    pub(crate) fn shift_to_redo(&mut self) {
        if let Some(e) = self.done.pop_back() {
            self.undone.push(e);
        }
        self.last_commit = None;
    }

    pub(crate) fn shift_to_done(&mut self) {
        if let Some(mut e) = self.undone.pop() {
            e.sealed = true;
            self.done.push_back(e);
        }
        self.last_commit = None;
    }

    /// After a divergence (a bug), the history can no longer be trusted.
    pub(crate) fn clear(&mut self) {
        let current = self.current_state();
        self.base_state = current;
        if self.saved != Some(current) {
            self.saved = None;
        }
        self.done.clear();
        self.undone.clear();
        self.bytes = 0;
        self.gesture = None;
        self.last_commit = None;
    }
}
