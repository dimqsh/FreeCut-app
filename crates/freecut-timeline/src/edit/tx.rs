//! Recording transactions.
//!
//! A planner mutates only through [`Tx::push`], which applies each primitive
//! to a working copy and records it. Later steps of a compound edit see the
//! earlier ones, and the recorded list is exactly what was done.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::change::scope_of;
use super::{Change, ChangeSet, EditCtx, EditError, Label, Notice, Plan};
use crate::ids::{ClipId, GroupId, IdCursor, LinkId, MediaId, SeqId, TrackId};
use crate::model::{Clip, ClipLoc, Document, Scope, Sequence, ValidateCtx};
use crate::time::{FrameRate, Tick};

pub(crate) struct Tx<'a> {
    base: &'a Document,
    pub(crate) ctx: &'a EditCtx,
    work: Document,
    changes: Vec<Change>,
    pub(crate) ids: IdCursor,
    notices: Vec<Notice>,
    selection_after: Option<Vec<ClipId>>,
    imported: Vec<MediaId>,
    /// Whether [`Tx::finish`] runs the attached-items pass (TL-029).
    follow: bool,
    /// Clips the op positioned itself, even where they didn't move: the
    /// attached-items pass leaves them where they are and reattaches them.
    placed: BTreeSet<ClipId>,
}

impl<'a> Tx<'a> {
    pub(crate) fn new(base: &'a Document, ctx: &'a EditCtx, ids: IdCursor) -> Tx<'a> {
        Tx {
            base,
            ctx,
            work: base.clone(),
            changes: Vec::new(),
            ids,
            notices: Vec::new(),
            selection_after: None,
            imported: Vec::new(),
            follow: true,
            placed: BTreeSet::new(),
        }
    }

    /// The document as it was before this edit.
    pub(crate) fn base(&self) -> &'a Document {
        self.base
    }

    /// What the edit has touched so far.
    pub(crate) fn scope(&self) -> Scope {
        scope_of(&self.changes)
    }

    /// Marks `ids` as positioned by the op (a move's partners that keep
    /// their place, TL-057): the attached-items pass doesn't move them.
    pub(crate) fn placed(&mut self, ids: impl IntoIterator<Item = ClipId>) {
        self.placed.extend(ids);
    }

    /// The clips marked with [`Tx::placed`].
    pub(crate) fn placed_ids(&self) -> &BTreeSet<ClipId> {
        &self.placed
    }

    /// Skips the attached-items pass: the op recomputed attachments itself.
    pub(crate) fn skip_follow(&mut self) {
        self.follow = false;
    }

    pub(crate) fn work(&self) -> &Document {
        &self.work
    }

    /// The frame grid of the document being edited (it can change within
    /// a transaction, through a settings change).
    pub(crate) fn rate(&self) -> FrameRate {
        self.work.settings().frame_rate
    }

    /// The media's last known length, from the library.
    pub(crate) fn media_duration(&self, id: MediaId) -> Option<Tick> {
        self.work.library().duration(id)
    }

    pub(crate) fn seq(&self, id: SeqId) -> Result<&Sequence, EditError> {
        self.work.sequence(id).ok_or(EditError::StaleSequence(id))
    }

    pub(crate) fn clip(&self, id: ClipId) -> Result<Arc<Clip>, EditError> {
        self.work.clip(id).cloned().ok_or(EditError::StaleClip(id))
    }

    pub(crate) fn locate(&self, id: ClipId) -> Result<ClipLoc, EditError> {
        self.work.find_clip(id).ok_or(EditError::StaleClip(id))
    }

    /// Records `change` and applies it to the working copy. A failure here
    /// is a planner bug.
    pub(crate) fn push(&mut self, change: Change) -> Result<(), EditError> {
        self.work.apply(&change).map_err(EditError::Internal)?;
        self.changes.push(change);
        Ok(())
    }

    pub(crate) fn notice(&mut self, notice: Notice) {
        self.notices.push(notice);
    }

    pub(crate) fn select(&mut self, ids: Vec<ClipId>) {
        self.selection_after = Some(ids);
    }

    pub(crate) fn created_media(&mut self, id: MediaId) {
        self.imported.push(id);
    }

    /// Runs the end-of-edit passes, validates what the edit touched, and
    /// produces the plan.
    pub(crate) fn finish(mut self, label: Label) -> Result<Plan, EditError> {
        if self.follow {
            super::follow::follow_owners(&mut self)?;
        }
        if scope_of(&self.changes).relations {
            self.dissolve_small_relations()?;
        }
        self.remove_emptied_tracks()?;
        let changes = ChangeSet::new(self.changes);
        let high_water = self.ids.high_water();
        let vctx = ValidateCtx {
            limits: self.ctx.limits,
            id_high_water: Some(high_water),
        };
        self.work
            .validate_scope(&vctx, &changes.scope())
            .map_err(EditError::Invariant)?;
        Ok(Plan::new(
            label,
            changes,
            self.notices,
            self.selection_after,
            self.work,
            self.imported,
            high_water,
        ))
    }

    /// Groups and links need at least two members (TL-039, TL-030). A link
    /// holding detached sound also needs the video clip whose sound it is:
    /// once this edit removed every such video, the link is dangling and is
    /// cleared from the sound pieces left, however many there are (TL-030).
    fn dissolve_small_relations(&mut self) -> Result<(), EditError> {
        let mut groups: BTreeMap<GroupId, Vec<ClipId>> = BTreeMap::new();
        let mut links: BTreeMap<LinkId, Vec<ClipId>> = BTreeMap::new();
        let mut detached: BTreeSet<LinkId> = BTreeSet::new();
        for seq in self.work.sequences() {
            for (_, track) in seq.tracks() {
                for clip in track.clips() {
                    if let Some(g) = clip.group {
                        groups.entry(g).or_default().push(clip.id);
                    }
                    if let Some(l) = clip.link {
                        links.entry(l).or_default().push(clip.id);
                    }
                    if let Some(l) = clip.audio.detached() {
                        detached.insert(l);
                    }
                }
            }
        }
        let detached_before = detached_links(self.base);
        let dangling: BTreeSet<ClipId> = links
            .iter()
            .filter(|(l, _)| detached_before.contains(l) && !detached.contains(l))
            .flat_map(|(_, members)| members.iter().copied())
            .collect();
        let lonely_groups = lonely(groups);
        let lonely_links: BTreeSet<ClipId> = lonely(links).union(&dangling).copied().collect();
        for id in lonely_groups.union(&lonely_links) {
            let loc = self.locate(*id)?;
            let before = self.clip(*id)?;
            let mut after = (*before).clone();
            if lonely_groups.contains(id) {
                after.group = None;
            }
            if lonely_links.contains(id) {
                after.link = None;
            }
            self.push(Change::ReplaceClip {
                seq: loc.seq,
                track: loc.track,
                before,
                after: Arc::new(after),
            })?;
        }
        Ok(())
    }

    /// Removes every non-main, unlocked track this edit emptied (TL-006).
    /// Tracks that were already empty (for example ones the user just added)
    /// stay.
    fn remove_emptied_tracks(&mut self) -> Result<(), EditError> {
        let mut doomed: Vec<(SeqId, TrackId)> = Vec::new();
        for seq in self.work.sequences() {
            let Some(base_seq) = self.base.sequence(seq.id) else {
                continue;
            };
            for (slot, track) in seq.tracks() {
                if slot.is_main() || !track.is_empty() || track.is_locked() {
                    continue;
                }
                let was_full = base_seq
                    .find_track(track.id)
                    .is_some_and(|(_, t)| !t.is_empty());
                if was_full {
                    doomed.push((seq.id, track.id));
                }
            }
        }
        let count = doomed.len();
        for (seq, track) in doomed {
            // Re-resolve the slot each time: earlier removals shift indices.
            let (slot, snapshot) = self
                .seq(seq)?
                .find_track(track)
                .map(|(slot, t)| (slot, Arc::clone(t)))
                .ok_or(EditError::StaleTrack(track))?;
            self.push(Change::RemoveTrack {
                seq,
                slot,
                track: snapshot,
            })?;
        }
        if count > 0 {
            self.notice(Notice::RemovedEmptyTracks(
                u32::try_from(count).unwrap_or(u32::MAX),
            ));
        }
        Ok(())
    }
}

/// The links holding the detached sound of some video clip of `doc`.
fn detached_links(doc: &Document) -> BTreeSet<LinkId> {
    doc.sequences()
        .flat_map(|seq| seq.tracks())
        .flat_map(|(_, track)| track.clips().iter())
        .filter_map(|clip| clip.audio.detached())
        .collect()
}

/// Members of relations with fewer than two members.
fn lonely<K>(members: BTreeMap<K, Vec<ClipId>>) -> BTreeSet<ClipId> {
    members
        .into_values()
        .filter(|v| v.len() < 2)
        .flatten()
        .collect()
}
