//! Adding media to the timeline (TL-012).

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::edit::policy::{self, MainItem};
use crate::edit::tx::Tx;
use crate::edit::{EditError, Label, OpKind, Plan};
use crate::ids::{MediaId, SeqId, TrackId};
use crate::model::{
    Clip, ClipContent, EmbeddedAudio, GeneratedKind, Lane, MediaKind, Sequence, StreamSel,
    TextClipData, Track, TrackKind,
};
use crate::time::{Rounding, Tick, TimeRange};

/// A clip to create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewClip {
    pub content: ClipContent,
    /// Length on the timeline. For media it equals the source length.
    pub duration: Tick,
    pub name: Option<String>,
}

impl NewClip {
    /// Time-based media showing `source`.
    pub fn media(asset: MediaId, kind: MediaKind, source: TimeRange) -> NewClip {
        NewClip {
            content: ClipContent::Media {
                asset,
                kind,
                source,
                stream: None,
            },
            duration: source.duration(),
            name: None,
        }
    }

    pub fn image(asset: MediaId, duration: Tick) -> NewClip {
        NewClip {
            content: ClipContent::Image { asset },
            duration,
            name: None,
        }
    }

    /// A text clip (TX-001). It lands on a `Text` track; with
    /// [`InsertTarget::FirstFree`] the first unlocked one free over its
    /// span, else a new one on top of the upper band. For the default
    /// length use [`crate::model::default_text_duration`]; any length is
    /// rounded to the frame grid.
    pub fn text(data: TextClipData, duration: Tick) -> NewClip {
        NewClip {
            content: ClipContent::Text(Arc::new(data)),
            duration,
            name: None,
        }
    }

    pub fn generated(kind: GeneratedKind, duration: Tick) -> NewClip {
        NewClip {
            content: ClipContent::Generated { kind },
            duration,
            name: None,
        }
    }
}

/// Where inserted clips go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertTarget {
    /// Onto this track, or the nearest compatible one if its kind doesn't fit.
    Track(TrackId),
    /// Onto a new track.
    NewTrack,
    /// Onto the first compatible, unlocked track that is free over the whole
    /// span, else a new track of that kind: audio tries the audio tracks top
    /// down, visual items the overlay tracks bottom up, never the main track
    /// (TL-012 "Add as overlay", AU-030).
    FirstFree,
}

/// Places `clips` back to back starting at `at` (TL-012). On the magnetic
/// main track they go in at the cut nearest `at` and push later clips.
///
/// Media without a picture always becomes audio clips on audio tracks,
/// whatever kind was asked for (AU-001). Video clips of media without sound
/// carry no sound settings (AU-002).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertReq {
    pub seq: SeqId,
    pub target: InsertTarget,
    pub at: Tick,
    pub clips: Vec<NewClip>,
}

pub(crate) fn insert(mut tx: Tx<'_>, req: &InsertReq) -> Result<Plan, EditError> {
    if req.clips.is_empty() {
        return Err(EditError::NothingSelected);
    }
    let mut clips = Vec::with_capacity(req.clips.len());
    for new in &req.clips {
        let (content, duration, audio) = normalize(&tx, new)?;
        let id = tx.ids.clip()?;
        let mut clip = Clip::new(id, TimeRange::new(Tick::ZERO, duration)?, content);
        clip.name.clone_from(&new.name);
        if let Some(audio) = audio {
            clip.audio = audio;
        }
        clips.push(clip);
    }
    let kind = TrackKind::for_content(&clips[0].content);
    if clips
        .iter()
        .any(|c| TrackKind::for_content(&c.content) != kind)
    {
        return Err(EditError::MixedKinds);
    }
    let ids: Vec<_> = clips.iter().map(|c| c.id).collect();

    let at = tx.rate().quantize(req.at)?.max(Tick::ZERO);
    let total = clips
        .iter()
        .try_fold(Tick::ZERO, |acc, c| acc.checked_add(c.duration()))?;
    let span = TimeRange::new(at, total)?;
    let target = resolve_target(&mut tx, req.seq, req.target, kind, span)?;
    let seq = tx.seq(req.seq)?;
    let (slot, track) = seq
        .find_track(target)
        .ok_or(EditError::StaleTrack(target))?;

    if slot.is_main() && seq.flags.main_magnet {
        policy::require_unlocked(track, slot)?;
        let layout = policy::main_layout(seq)?;
        let cut = policy::cut_index(&layout, at);
        let mut order: Vec<MainItem> = layout
            .iter()
            .map(|(id, _, _)| MainItem::Existing(*id))
            .collect();
        order.splice(
            cut..cut,
            clips.into_iter().map(|c| MainItem::New(Box::new(c))),
        );
        policy::layout_main(&mut tx, req.seq, order)?;
    } else {
        let mut pos = at;
        let mut placed = Vec::with_capacity(clips.len());
        for clip in clips {
            let moved = clip.moved_to(pos)?;
            pos = moved.end();
            placed.push(moved);
        }
        policy::place_clips(&mut tx, req.seq, target, placed)?;
    }
    tx.select(ids.clone());
    tx.finish(Label {
        op: OpKind::InsertClips,
        count: u32::try_from(ids.len()).unwrap_or(u32::MAX),
    })
}

/// Checks the content against the media and puts its length on the frame
/// grid. Media lengths round down so a clip never reads past its source.
/// Also picks the media kind and, when it differs from the default, the
/// sound settings.
fn normalize(
    tx: &Tx<'_>,
    new: &NewClip,
) -> Result<(ClipContent, Tick, Option<EmbeddedAudio>), EditError> {
    let rate = tx.rate();
    match &new.content {
        ClipContent::Media {
            asset,
            kind,
            source,
            stream,
        } => {
            let info = &tx
                .work()
                .library()
                .get(*asset)
                .ok_or(EditError::UnknownMedia(*asset))?
                .info;
            let len = info.duration;
            let kind = if !info.has_video() && info.has_audio() {
                MediaKind::Audio
            } else {
                *kind
            };
            let audio =
                (kind == MediaKind::Video && !info.has_audio()).then_some(EmbeddedAudio::None);
            if source.start().is_negative() || source.end() > len {
                return Err(EditError::SourceOutOfRange);
            }
            if let Some(StreamSel::Index(index)) = *stream
                && info.audio_stream(index).is_none()
            {
                return Err(EditError::UnknownStream {
                    media: *asset,
                    stream: index,
                });
            }
            let duration = rate.round_to_grid(source.duration(), Rounding::Floor)?;
            if !duration.is_positive() {
                return Err(EditError::TooShort);
            }
            Ok((
                ClipContent::Media {
                    asset: *asset,
                    kind,
                    source: source.with_duration(duration)?,
                    stream: *stream,
                },
                duration,
                audio,
            ))
        }
        other => {
            let duration = rate.quantize(new.duration)?;
            if !duration.is_positive() {
                return Err(EditError::TooShort);
            }
            Ok((other.clone(), duration, None))
        }
    }
}

/// The track inserted clips of `kind` land on: the requested track if it
/// fits the kind, else the nearest compatible track in the same lane, else a
/// new track (TL-004: items are redirected, never converted).
fn resolve_target(
    tx: &mut Tx<'_>,
    seq_id: SeqId,
    target: InsertTarget,
    kind: TrackKind,
    span: TimeRange,
) -> Result<TrackId, EditError> {
    let seq = tx.seq(seq_id)?;
    match target {
        InsertTarget::Track(id) => {
            let (slot, track) = seq.find_track(id).ok_or(EditError::StaleTrack(id))?;
            if track.kind == kind {
                return Ok(id);
            }
            if let Some(near) = nearest_of_kind(seq, kind, slot.lane, slot.index, span) {
                return Ok(near);
            }
            let slot = policy::slot_next_to(seq, kind, slot);
            policy::add_track_at(tx, seq_id, kind, slot)
        }
        InsertTarget::NewTrack => {
            let slot = policy::slot_for_new_track(seq, kind, None);
            policy::add_track_at(tx, seq_id, kind, slot)
        }
        InsertTarget::FirstFree => {
            let free = |t: &&Arc<Track>| {
                t.kind == kind && !t.is_locked() && policy::fits(t, &[span], &BTreeSet::new())
            };
            let found = if kind.is_audio() {
                seq.audio_tracks().iter().find(free)
            } else {
                // Index 0 is the main track, which this never uses.
                seq.visual_tracks().iter().skip(1).find(free)
            };
            if let Some(track) = found {
                return Ok(track.id);
            }
            let slot = policy::slot_for_new_track(seq, kind, None);
            policy::add_track_at(tx, seq_id, kind, slot)
        }
    }
}

/// The compatible track nearest the drop position: preferably one that is
/// unlocked and free over `span` (TL-012), else simply the nearest one (the
/// placement then overflows to a new track above it).
fn nearest_of_kind(
    seq: &Sequence,
    kind: TrackKind,
    lane: Lane,
    index: usize,
    span: TimeRange,
) -> Option<TrackId> {
    let lane_tracks = seq.lane(if kind.is_audio() {
        Lane::Audio
    } else {
        Lane::Visual
    });
    let anchor = if (lane == Lane::Audio) == kind.is_audio() {
        index
    } else {
        0
    };
    let candidates = || {
        lane_tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.kind == kind)
    };
    let free = candidates()
        .filter(|(_, t)| !t.is_locked() && policy::fits(t, &[span], &BTreeSet::new()))
        .min_by_key(|(i, _)| i.abs_diff(anchor));
    free.or_else(|| candidates().min_by_key(|(i, _)| i.abs_diff(anchor)))
        .map(|(_, t)| t.id)
}
