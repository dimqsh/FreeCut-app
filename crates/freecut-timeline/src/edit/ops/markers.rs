//! Markers (TL-037): add, edit, move and delete timeline and clip markers.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::edit::policy;
use crate::edit::tx::Tx;
use crate::edit::{Change, EditError, Label, Notice, OpKind, Plan};
use crate::ids::{ClipId, MarkerId, SeqId};
use crate::model::{
    Clip, ClipLoc, MAX_MARKER_NAME_CHARS, MAX_MARKER_NOTE_CHARS, Marker, MarkerRef, Markers,
    PaletteColor,
};
use crate::time::{MarkRange, Tick};

/// Who a new marker belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkerOwner {
    /// The `M` rule against this selection: a clip marker when exactly one
    /// clip is selected and the time is inside it, else a timeline marker.
    Auto(Vec<ClipId>),
    Timeline,
    Clip(ClipId),
}

/// Changes to one marker; `None` fields stay as they are.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MarkerPatch {
    /// Control characters are dropped and the rest is cut at 120 characters.
    pub name: Option<String>,
    /// Cut at 2,000 characters.
    pub note: Option<String>,
    pub color: Option<PaletteColor>,
    /// Frame-quantized; 0 (or less) makes a point marker.
    pub duration: Option<Tick>,
}

/// Which markers to delete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkerSelector {
    Refs(Vec<MarkerRef>),
    /// Every marker of this colour, timeline and clip markers alike.
    Color(PaletteColor),
    /// Every marker on this clip.
    OnClip(ClipId),
    /// Every timeline marker.
    AllTimeline,
}

/// A clip that can take marker edits: found in `seq` and not locked.
fn editable_clip(tx: &Tx<'_>, seq: SeqId, id: ClipId) -> Result<(ClipLoc, Arc<Clip>), EditError> {
    let loc = tx.locate(id)?;
    if loc.seq != seq {
        return Err(EditError::StaleClip(id));
    }
    let track = tx
        .seq(seq)?
        .track_at(loc.slot)
        .ok_or(EditError::StaleTrack(loc.track))?;
    policy::require_unlocked(track, loc.slot)?;
    Ok((loc, tx.clip(id)?))
}

fn set_clip_markers(
    tx: &mut Tx<'_>,
    loc: ClipLoc,
    clip: Arc<Clip>,
    markers: Markers,
) -> Result<(), EditError> {
    let mut after = (*clip).clone();
    after.markers = markers;
    tx.push(Change::ReplaceClip {
        seq: loc.seq,
        track: loc.track,
        before: clip,
        after: Arc::new(after),
    })
}

fn set_seq_markers(tx: &mut Tx<'_>, seq: SeqId, markers: Markers) -> Result<(), EditError> {
    let before = tx.seq(seq)?.markers.clone();
    tx.push(Change::SetSeqMarkers {
        seq,
        before,
        after: markers,
    })
}

/// Adds a point marker at `at` (frame-quantized), named "Marker N" for its
/// owner, in `color` (Blue by default). It is reported with
/// [`Notice::AddedMarker`] so the app can select it.
pub(crate) fn add_marker(
    mut tx: Tx<'_>,
    seq: SeqId,
    at: Tick,
    owner: &MarkerOwner,
    color: Option<PaletteColor>,
) -> Result<Plan, EditError> {
    let at = tx.rate().quantize(at)?;
    let owner_clip = match owner {
        MarkerOwner::Timeline => None,
        MarkerOwner::Clip(id) => Some(*id),
        MarkerOwner::Auto(selection) => match selection.as_slice() {
            [id] if tx.work().clip(*id).is_some_and(|c| c.range.contains(at))
                && tx.locate(*id).is_ok_and(|l| l.seq == seq) =>
            {
                Some(*id)
            }
            _ => None,
        },
    };
    let id = tx.ids.marker()?;
    let mut marker = Marker::new(id, at);
    marker.color = color.unwrap_or_default();
    let added = match owner_clip {
        None => {
            if at.is_negative() {
                return Err(EditError::BeforeZero);
            }
            let list = tx.seq(seq)?.markers.clone();
            marker.name = list.next_name();
            set_seq_markers(&mut tx, seq, list.with(marker))?;
            MarkerRef::Timeline(id)
        }
        Some(clip_id) => {
            let (loc, clip) = editable_clip(&tx, seq, clip_id)?;
            if !clip.range.contains(at) {
                return Err(EditError::NotUnderPlayhead);
            }
            let time = clip
                .content_time_at(at)
                .ok_or(EditError::StaleClip(clip_id))?;
            marker.at = MarkRange::point(time);
            marker.name = clip.markers.next_name();
            let list = clip.markers.with(marker);
            set_clip_markers(&mut tx, loc, clip, list)?;
            MarkerRef::Clip(clip_id, id)
        }
    };
    tx.notice(Notice::AddedMarker(added));
    tx.finish(Label {
        op: OpKind::AddMarker,
        count: 1,
    })
}

/// Where a marker lives: its clip for clip markers, `None` for the timeline.
type Owner = Option<(ClipLoc, Arc<Clip>)>;

/// The marker `r` points at, with its owner.
fn find_marker(tx: &Tx<'_>, seq: SeqId, r: MarkerRef) -> Result<(Marker, Owner), EditError> {
    match r {
        MarkerRef::Timeline(id) => {
            let m = tx
                .seq(seq)?
                .markers
                .get(id)
                .cloned()
                .ok_or(EditError::StaleMarker(id))?;
            Ok((m, None))
        }
        MarkerRef::Clip(clip_id, id) => {
            let (loc, clip) = editable_clip(tx, seq, clip_id)?;
            let m = clip
                .markers
                .get(id)
                .cloned()
                .ok_or(EditError::StaleMarker(id))?;
            Ok((m, Some((loc, clip))))
        }
    }
}

/// Puts `marker` back where `owner` says, replacing the one with its id.
fn store(tx: &mut Tx<'_>, seq: SeqId, owner: Owner, marker: Marker) -> Result<(), EditError> {
    match owner {
        None => {
            let list = tx.seq(seq)?.markers.with(marker);
            set_seq_markers(tx, seq, list)
        }
        Some((loc, clip)) => {
            let list = clip.markers.with(marker);
            set_clip_markers(tx, loc, clip, list)
        }
    }
}

fn clean_name(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(MAX_MARKER_NAME_CHARS)
        .collect()
}

/// Applies `patch` to one marker as one entry; an unchanged marker records
/// nothing.
pub(crate) fn edit_marker(
    mut tx: Tx<'_>,
    seq: SeqId,
    r: MarkerRef,
    patch: &MarkerPatch,
) -> Result<Plan, EditError> {
    let (before, owner) = find_marker(&tx, seq, r)?;
    let mut after = before.clone();
    if let Some(name) = &patch.name {
        after.name = clean_name(name);
    }
    if let Some(note) = &patch.note {
        after.note = note.chars().take(MAX_MARKER_NOTE_CHARS).collect();
    }
    if let Some(color) = patch.color {
        after.color = color;
    }
    if let Some(duration) = patch.duration {
        let d = tx.rate().quantize(duration)?.max(Tick::ZERO);
        after.at = after.at.with_duration(d)?;
    }
    let changed = after != before;
    if changed {
        store(&mut tx, seq, owner, after)?;
    }
    tx.finish(Label {
        op: OpKind::EditMarker,
        count: u32::from(changed),
    })
}

/// Moves a marker so it starts at sequence time `to` (frame-quantized). A
/// timeline marker stops at 0; a clip marker stays on the content its clip
/// shows (its last frame at most).
pub(crate) fn move_marker(
    mut tx: Tx<'_>,
    seq: SeqId,
    r: MarkerRef,
    to: Tick,
) -> Result<Plan, EditError> {
    let rate = tx.rate();
    let to = rate.quantize(to)?;
    let (before, owner) = find_marker(&tx, seq, r)?;
    let start = match &owner {
        None => to.max(Tick::ZERO),
        Some((_, clip)) => {
            let range = clip.content_range().ok_or(EditError::StaleClip(clip.id))?;
            let offset = to.checked_sub(clip.start())?;
            let wanted = range.start().checked_add(offset)?;
            let last = range
                .end()
                .checked_sub(rate.frame_ticks())?
                .max(range.start());
            wanted.clamp(range.start(), last)
        }
    };
    let mut after = before.clone();
    after.at = after.at.with_start(start)?;
    let changed = after != before;
    if changed {
        store(&mut tx, seq, owner, after)?;
    }
    tx.finish(Label {
        op: OpKind::MoveMarker,
        count: u32::from(changed),
    })
}

/// Deletes the selected markers as one entry. Clip markers on locked tracks
/// are left alone (a clip named by [`MarkerSelector::OnClip`] on a locked
/// track refuses). Nothing to delete records nothing.
pub(crate) fn delete_markers(
    mut tx: Tx<'_>,
    seq: SeqId,
    which: &MarkerSelector,
) -> Result<Plan, EditError> {
    let s = tx.seq(seq)?;
    // Timeline marker ids to drop, and per clip the marker ids to drop.
    let mut timeline: BTreeSet<MarkerId> = BTreeSet::new();
    let mut on_clips: BTreeMap<ClipId, BTreeSet<MarkerId>> = BTreeMap::new();
    match which {
        MarkerSelector::Refs(refs) => {
            for r in refs {
                match *r {
                    MarkerRef::Timeline(id) => {
                        if s.markers.get(id).is_none() {
                            return Err(EditError::StaleMarker(id));
                        }
                        timeline.insert(id);
                    }
                    MarkerRef::Clip(clip, id) => {
                        let c = tx.work().clip(clip).ok_or(EditError::StaleClip(clip))?;
                        if c.markers.get(id).is_none() {
                            return Err(EditError::StaleMarker(id));
                        }
                        on_clips.entry(clip).or_default().insert(id);
                    }
                }
            }
        }
        MarkerSelector::Color(color) => {
            timeline.extend(s.markers.iter().filter(|m| m.color == *color).map(|m| m.id));
            for (_, track) in s.tracks() {
                for clip in track.clips() {
                    let ids: BTreeSet<_> = clip
                        .markers
                        .iter()
                        .filter(|m| m.color == *color)
                        .map(|m| m.id)
                        .collect();
                    if !ids.is_empty() {
                        on_clips.insert(clip.id, ids);
                    }
                }
            }
        }
        MarkerSelector::OnClip(clip) => {
            let (_, c) = editable_clip(&tx, seq, *clip)?;
            on_clips.insert(*clip, c.markers.iter().map(|m| m.id).collect());
        }
        MarkerSelector::AllTimeline => {
            timeline.extend(s.markers.iter().map(|m| m.id));
        }
    }
    let mut count = 0usize;
    if !timeline.is_empty() {
        let list = tx.seq(seq)?.markers.without(|m| timeline.contains(&m.id));
        count += timeline.len();
        set_seq_markers(&mut tx, seq, list)?;
    }
    for (clip, ids) in on_clips {
        let Ok((loc, c)) = editable_clip(&tx, seq, clip) else {
            continue;
        };
        if ids.is_empty() {
            continue;
        }
        count += ids.len();
        let list = c.markers.without(|m| ids.contains(&m.id));
        set_clip_markers(&mut tx, loc, c, list)?;
    }
    tx.finish(Label {
        op: OpKind::DeleteMarkers,
        count: u32::try_from(count).unwrap_or(u32::MAX),
    })
}
