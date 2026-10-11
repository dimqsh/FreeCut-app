//! Linked sound out of sync with its video (TL-030): the offset badge and
//! the Resync command.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::audio::free_audio_track;
use super::resolve_clips;
use crate::edit::policy;
use crate::edit::tx::Tx;
use crate::edit::{Change, EditError, Label, OpKind, Plan};
use crate::ids::{ClipId, LinkId, SeqId};
use crate::model::{Clip, ClipContent, Document, MediaKind, Sequence};
use crate::time::{FrameRate, Rounding, Tick, div_round};

/// How far a linked clip has slipped against its partner (TL-030).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncOffset {
    /// The offset in sequence frames, for the badge ("+12f"): rounded to the
    /// nearest frame, exact halves toward zero. Positive when the sound is
    /// late. It can be 0 for an offset of less than half a frame.
    pub frames: i64,
    /// The exact offset: the sound's (start − source start) minus the
    /// video's. Never zero.
    pub ticks: Tick,
    /// The clip the offset is measured against.
    pub partner: ClipId,
}

/// The out-of-sync offset of a linked clip (TL-030): `None` when the clip
/// isn't a linked media clip of `seq`, has no partner, or is exactly in
/// sync.
///
/// Partners share the clip's link: an audio clip's partners are the video
/// clips of the link (pieces, when the video was split), a video's are the
/// audio clips of the link (detached sound and streams added as clips,
/// AU-038). An audio clip reports its offset against the video piece it is
/// closest to being in sync with; a video reports the largest offset among
/// its audio partners, against itself (ties: the earlier partner). A scan of
/// one sequence; [`sync_offsets`] does every clip in one pass.
pub fn sync_offset(doc: &Document, seq: SeqId, clip: ClipId) -> Option<SyncOffset> {
    if doc.find_clip(clip)?.seq != seq {
        return None;
    }
    let c = doc.clip(clip)?;
    let link = c.link?;
    let members = link_members(doc.sequence(seq)?, link);
    offset_among(c, &members, doc.settings().frame_rate)
}

/// [`sync_offset`] of every out-of-sync linked clip of `seq`, in one pass
/// over the sequence (for drawing badges).
pub fn sync_offsets(doc: &Document, seq: SeqId) -> BTreeMap<ClipId, SyncOffset> {
    let Some(s) = doc.sequence(seq) else {
        return BTreeMap::new();
    };
    let rate = doc.settings().frame_rate;
    let mut links: BTreeMap<LinkId, Vec<&Arc<Clip>>> = BTreeMap::new();
    for (_, track) in s.tracks() {
        for clip in track.clips() {
            if let Some(link) = clip.link {
                links.entry(link).or_default().push(clip);
            }
        }
    }
    let mut out = BTreeMap::new();
    for members in links.values() {
        for clip in members {
            if let Some(offset) = offset_among(clip, members, rate) {
                out.insert(clip.id, offset);
            }
        }
    }
    out
}

/// The clips of `seq` carrying `link`, in track order, then by start.
fn link_members(seq: &Sequence, link: LinkId) -> Vec<&Arc<Clip>> {
    seq.tracks()
        .flat_map(|(_, t)| t.clips().iter())
        .filter(|c| c.link == Some(link))
        .collect()
}

fn media_kind(clip: &Clip) -> Option<MediaKind> {
    match clip.content {
        ClipContent::Media { kind, .. } => Some(kind),
        _ => None,
    }
}

/// Where the clip's source time 0 sits on the timeline.
fn anchor(clip: &Clip) -> Option<i128> {
    let source = clip.content.source()?;
    Some(i128::from(clip.start().raw()) - i128::from(source.start().raw()))
}

/// `audio`'s offset against `video`, in ticks (0 when in sync).
fn offset_between(audio: &Clip, video: &Clip) -> Option<Tick> {
    let ticks = anchor(audio)? - anchor(video)?;
    i64::try_from(ticks).ok().map(Tick::from_raw)
}

fn offset_among(clip: &Clip, members: &[&Arc<Clip>], rate: FrameRate) -> Option<SyncOffset> {
    let kind = media_kind(clip)?;
    let partners = members
        .iter()
        .filter(|m| m.id != clip.id && media_kind(m).is_some_and(|k| k != kind));
    let measured = partners.filter_map(|p| {
        let ticks = match kind {
            MediaKind::Audio => offset_between(clip, p)?,
            MediaKind::Video => offset_between(p, clip)?,
        };
        Some((ticks, p))
    });
    let key = |(ticks, p): &(Tick, &&Arc<Clip>)| (ticks.raw().unsigned_abs(), p.start(), p.id);
    let (ticks, partner) = match kind {
        MediaKind::Audio => measured.min_by_key(key)?,
        // The largest offset; among equals the earliest partner.
        MediaKind::Video => measured.min_by_key(|m| {
            let (abs, start, id) = key(m);
            (core::cmp::Reverse(abs), start, id)
        })?,
    };
    (ticks != Tick::ZERO).then(|| SyncOffset {
        frames: frames_of(ticks, rate),
        ticks,
        partner: partner.id,
    })
}

/// `ticks` in frames of `rate`: nearest, exact halves toward zero.
fn frames_of(ticks: Tick, rate: FrameRate) -> i64 {
    let frame = i128::from(rate.frame_ticks().raw());
    let n = div_round(
        i128::from(ticks.raw().unsigned_abs()),
        frame,
        Rounding::NearestTiesDown,
    );
    let n = i64::try_from(n).unwrap_or(i64::MAX);
    if ticks.is_negative() { -n } else { n }
}

/// Resync (TL-030): moves each out-of-sync audio clip among `ids` back by
/// its offset, and for each listed video every out-of-sync audio partner by
/// its offset against that video. A clip stays on its track when the target
/// span is free there, else it goes to the first free unlocked audio track
/// (or a new one, TL-006). Refused when nothing listed is out of sync, when
/// a clip would start before 0, or when a moving clip is on a locked track.
/// One undo entry.
pub(crate) fn resync(mut tx: Tx<'_>, seq: SeqId, ids: &[ClipId]) -> Result<Plan, EditError> {
    if ids.is_empty() {
        return Err(EditError::NothingSelected);
    }
    let resolved = resolve_clips(&tx, seq, ids)?;
    let s = tx.seq(seq)?;
    let mut moves: Vec<(Arc<Clip>, Tick)> = Vec::new();
    let mut seen = BTreeSet::new();
    for target in &resolved {
        let clip = &target.clip;
        let Some(link) = clip.link else { continue };
        let members = link_members(s, link);
        match media_kind(clip) {
            Some(MediaKind::Audio) => {
                if let Some(o) = offset_among(clip, &members, tx.rate())
                    && seen.insert(clip.id)
                {
                    moves.push((Arc::clone(clip), o.ticks));
                }
            }
            Some(MediaKind::Video) => {
                for audio in members
                    .iter()
                    .filter(|m| media_kind(m) == Some(MediaKind::Audio))
                {
                    let Some(ticks) = offset_between(audio, clip) else {
                        continue;
                    };
                    if ticks != Tick::ZERO && seen.insert(audio.id) {
                        moves.push((Arc::clone(audio), ticks));
                    }
                }
            }
            None => {}
        }
    }
    if moves.is_empty() {
        return Err(EditError::NothingToResync);
    }
    let mut placed = Vec::with_capacity(moves.len());
    for (clip, offset) in &moves {
        let loc = tx.locate(clip.id)?;
        let track = tx
            .seq(seq)?
            .track_at(loc.slot)
            .ok_or(EditError::StaleTrack(loc.track))?;
        policy::require_unlocked(track, loc.slot)?;
        let start = clip.start().checked_sub(*offset)?;
        if start.is_negative() {
            return Err(EditError::BeforeZero);
        }
        placed.push((loc.track, Arc::clone(clip), clip.moved_to(start)?));
    }
    for (track, clip, _) in &placed {
        tx.push(Change::RemoveClip {
            seq,
            track: *track,
            clip: Arc::clone(clip),
        })?;
    }
    let count = u32::try_from(placed.len()).unwrap_or(u32::MAX);
    for (track, _, moved) in placed {
        let home = tx
            .seq(seq)?
            .find_track(track)
            .is_some_and(|(_, t)| policy::fits(t, &[moved.range], &BTreeSet::new()));
        let track = if home {
            track
        } else {
            free_audio_track(&mut tx, seq, moved.range)?
        };
        tx.push(Change::InsertClip {
            seq,
            track,
            clip: Arc::new(moved),
        })?;
    }
    tx.finish(Label {
        op: OpKind::Resync,
        count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_to_nearest_with_halves_toward_zero() {
        let rate = FrameRate::new(25, 1).unwrap();
        let frame = rate.frame_ticks().raw();
        let at = |ticks: i64| frames_of(Tick::from_raw(ticks), rate);
        assert_eq!(at(frame * 3), 3);
        assert_eq!(at(-frame * 3), -3);
        assert_eq!(at(frame / 2), 0);
        assert_eq!(at(-frame / 2), 0);
        assert_eq!(at(frame / 2 + 1), 1);
        assert_eq!(at(-(frame / 2 + 1)), -1);
        assert_eq!(at(frame * 3 + frame / 2), 3);
        assert_eq!(at(-(frame * 3 + frame / 2)), -3);
        assert_eq!(at(1), 0);
    }
}
