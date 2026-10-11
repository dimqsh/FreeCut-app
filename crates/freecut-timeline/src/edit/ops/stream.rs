//! Files with several audio streams (AU-038): choosing a clip's stream and
//! putting one stream on its own linked audio clip.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::library::audio_item;
use super::resolve_clips;
use crate::edit::policy;
use crate::edit::tx::Tx;
use crate::edit::{Change, EditError, Label, Notice, OpKind, Plan};
use crate::ids::{ClipId, LinkId, MediaId, SeqId, TrackId};
use crate::model::{
    Clip, ClipContent, Document, EmbeddedAudio, Library, MediaKind, MediaSource, Sequence,
    StreamSel, TrackKind, TrackSlot,
};
use crate::time::TimeRange;

/// Sets the stream selection of every listed media clip that plays its own
/// sound (AU-038). Skipped: clips without sound, clips whose sound is
/// detached (the stream belongs to the detached clips), for a chosen stream,
/// clips whose media lacks it, and clips whose new selection would play a
/// stream that another clip of their link already plays over an overlapping
/// span (see [`doubled_by`]; "All streams mixed" plays every stream), so a
/// stream is never doubled. Clips earlier in the request count with their
/// new selection. When nothing is left the edit is refused with the most
/// specific reason: the missing stream, then the doubled stream, then the
/// detached sound, then no sound. Locked tracks refuse it (TL-009).
pub(crate) fn set_audio_stream(
    mut tx: Tx<'_>,
    seq: SeqId,
    ids: &[ClipId],
    sel: Option<StreamSel>,
) -> Result<Plan, EditError> {
    if ids.is_empty() {
        return Err(EditError::NothingSelected);
    }
    let mut targets = Vec::new();
    let mut detached = false;
    let mut missing: Option<(MediaId, u32)> = None;
    let mut doubled: Option<EditError> = None;
    // The selections this request has already given to earlier targets.
    let mut chosen: BTreeMap<ClipId, Option<StreamSel>> = BTreeMap::new();
    for target in resolve_clips(&tx, seq, ids)? {
        let ClipContent::Media { asset, .. } = target.clip.content else {
            continue;
        };
        match target.clip.audio {
            EmbeddedAudio::Detached(_) => {
                detached = true;
                continue;
            }
            EmbeddedAudio::None => continue,
            EmbeddedAudio::Active(_) => {}
        }
        if let Some(StreamSel::Index(stream)) = sel {
            let has = tx
                .work()
                .library()
                .get(asset)
                .is_some_and(|m| m.info.audio_stream(stream).is_some());
            if !has {
                missing.get_or_insert((asset, stream));
                continue;
            }
        }
        // An unchanged selection changes nothing, so it can't double.
        if selection(&target.clip) != sel {
            let current = |c: &Clip| chosen.get(&c.id).copied().unwrap_or_else(|| selection(c));
            let found = doubled_by(
                tx.seq(seq)?,
                tx.work().library(),
                &target.clip,
                sel,
                current,
            );
            if let Some((by, stream)) = found {
                doubled.get_or_insert(EditError::StreamPlaysElsewhere {
                    clip: target.clip.id,
                    stream,
                    by,
                });
                continue;
            }
            chosen.insert(target.clip.id, sel);
        }
        targets.push(target);
    }
    if targets.is_empty() {
        return Err(match (missing, doubled, detached) {
            (Some((media, stream)), _, _) => EditError::UnknownStream { media, stream },
            (None, Some(doubled), _) => doubled,
            (None, None, true) => EditError::SoundDetached,
            (None, None, false) => EditError::NoClipAudio,
        });
    }
    let s = tx.seq(seq)?;
    for target in &targets {
        let track = s
            .track_at(target.loc.slot)
            .ok_or(EditError::StaleTrack(target.loc.track))?;
        policy::require_unlocked(track, target.loc.slot)?;
    }
    let mut count: u32 = 0;
    for target in targets {
        let mut clip = (*target.clip).clone();
        if let ClipContent::Media { stream, .. } = &mut clip.content {
            if *stream == sel {
                continue;
            }
            *stream = sel;
        }
        tx.push(Change::ReplaceClip {
            seq,
            track: target.loc.track,
            before: target.clip,
            after: Arc::new(clip),
        })?;
        count = count.saturating_add(1);
    }
    tx.finish(Label {
        op: OpKind::SetAudioStream,
        count,
    })
}

/// "Add stream as clip" (AU-038): an audio clip playing stream `stream` of
/// the video clip's file, through the extracted item of that stream (made
/// now, or reused as by an extract, AU-004), with the video's timeline and
/// source range and enabled state, default sound settings, and the video's
/// link (made now when it has none), so the out-of-sync badge works
/// (TL-030). It goes on the first free unlocked audio track below the
/// video's other linked sound (or from the top of the audio band), else on a
/// new audio track at the bottom of the band (TL-006). Refused with
/// `StreamAlreadyPlaying` when the stream already sounds with the video
/// (see [`already_plays`]), so it is never doubled, and with
/// `SourceOutOfRange` when the reused item (relinked on its own to a shorter
/// file) doesn't cover the video's source range.
pub(crate) fn add_stream_as_clip(
    mut tx: Tx<'_>,
    seq: SeqId,
    id: ClipId,
    stream: u32,
) -> Result<Plan, EditError> {
    let loc = tx.locate(id)?;
    if loc.seq != seq {
        return Err(EditError::StaleClip(id));
    }
    let video = tx.clip(id)?;
    let ClipContent::Media {
        asset,
        kind: MediaKind::Video,
        source,
        ..
    } = video.content
    else {
        return Err(EditError::NotVideo { clip: id });
    };
    let item = tx
        .work()
        .library()
        .get(asset)
        .cloned()
        .ok_or(EditError::UnknownMedia(asset))?;
    let info = &item.info;
    if !info.has_audio() || video.audio == EmbeddedAudio::None {
        return Err(EditError::NoSound(asset));
    }
    if info.audio_stream(stream).is_none() {
        return Err(EditError::UnknownStream {
            media: asset,
            stream,
        });
    }
    if info.audio.len() < 2 || item.source != MediaSource::File {
        return Err(EditError::SingleStream(asset));
    }
    let s = tx.seq(seq)?;
    if already_plays(s, tx.work().library(), &video, stream) {
        return Err(EditError::StreamAlreadyPlaying { clip: id, stream });
    }
    let track = s
        .track_at(loc.slot)
        .ok_or(EditError::StaleTrack(loc.track))?;
    policy::require_unlocked(track, loc.slot)?;

    let (audio_asset, _) = audio_item(&mut tx, &item, stream)?;
    // A reused extracted item relinked on its own to a shorter file can't
    // play the video's whole source range.
    let covers = tx
        .work()
        .library()
        .duration(audio_asset)
        .is_none_or(|len| source.end() <= len);
    if !covers {
        return Err(EditError::SourceOutOfRange);
    }
    // A detached video whose link dissolved keeps the link id of its sound;
    // reusing it keeps the two consistent (TL-030).
    let link = match video.link.or(video.audio.detached()) {
        Some(link) => link,
        None => tx.ids.link()?,
    };
    if video.link != Some(link) {
        let mut linked = (*video).clone();
        linked.link = Some(link);
        tx.push(Change::ReplaceClip {
            seq,
            track: loc.track,
            before: Arc::clone(&video),
            after: Arc::new(linked),
        })?;
    }
    let new_id = tx.ids.clip()?;
    let mut clip = Clip::new(
        new_id,
        video.range,
        ClipContent::Media {
            asset: audio_asset,
            kind: MediaKind::Audio,
            source,
            stream: None,
        },
    );
    clip.enabled = video.enabled;
    clip.link = Some(link);
    let track = track_below_link(&mut tx, seq, video.range, link)?;
    tx.push(Change::InsertClip {
        seq,
        track,
        clip: Arc::new(clip),
    })?;
    tx.select(vec![new_id]);
    tx.finish(Label {
        op: OpKind::AddStreamAsClip,
        count: 1,
    })
}

/// Whether container stream `stream` of video clip `video`'s file already
/// sounds with the video, so adding it as a clip would double it (AU-038):
/// through the video's own sound while it is active (its selection, "All
/// streams mixed" included), or through an audio clip of its link (or of its
/// detached link) that overlaps it in time: a piece of its detached sound,
/// by that clip's own selection, or a stream added as a clip before.
fn already_plays(seq: &Sequence, library: &Library, video: &Clip, stream: u32) -> bool {
    let Some(file) = video.content.asset() else {
        return false;
    };
    let plays = |c: &Clip| played(library, c, selection(c)).contains(&(file, stream));
    if video.audio.active().is_some() && plays(video) {
        return true;
    }
    let Some(link) = video.link.or(video.audio.detached()) else {
        return false;
    };
    sounding_partners(seq, link, video).any(|c| plays(c))
}

/// The clip that already plays a stream `clip` would play with selection
/// `sel`, so [`EditRequest::SetAudioStream`](crate::edit::EditRequest)
/// would skip it rather than double the stream (AU-038): another clip of its
/// link, with active sound and overlapping it in time, playing the same
/// stream of the same file (a stream added as a clip, or a piece of detached
/// sound). "All streams mixed" plays every stream. `None` when nothing
/// doubles, when `sel` is the clip's current selection (nothing would
/// change), or when `clip` isn't a media clip of `seq` with active sound.
/// Read-only and cheap (a scan of the clips overlapping `clip`), for the
/// app's pickers.
pub fn stream_doubled_by(
    doc: &Document,
    seq: SeqId,
    clip: ClipId,
    sel: Option<StreamSel>,
) -> Option<ClipId> {
    if doc.find_clip(clip)?.seq != seq {
        return None;
    }
    let c = doc.clip(clip)?;
    if selection(c) == sel {
        return None;
    }
    doubled_by(doc.sequence(seq)?, doc.library(), c, sel, selection).map(|(by, _)| by)
}

/// The first clip that would double a stream `clip` plays with selection
/// `sel` (AU-038), with the doubled container stream: another clip of its
/// link, with active sound, overlapping it in time and playing the same
/// (file, stream), where `sel_of` gives each other clip's selection. `None`
/// when `clip` has no active sound or no link.
fn doubled_by(
    seq: &Sequence,
    library: &Library,
    clip: &Clip,
    sel: Option<StreamSel>,
    sel_of: impl Fn(&Clip) -> Option<StreamSel>,
) -> Option<(ClipId, u32)> {
    let link = clip.link?;
    clip.audio.active()?;
    let mine = played(library, clip, sel);
    if mine.is_empty() {
        return None;
    }
    sounding_partners(seq, link, clip).find_map(|c| {
        played(library, c, sel_of(c))
            .into_iter()
            .find(|pair| mine.contains(pair))
            .map(|(_, stream)| (c.id, stream))
    })
}

/// The other clips of `link` with active sound that overlap `clip` in time.
fn sounding_partners<'a>(
    seq: &'a Sequence,
    link: LinkId,
    clip: &'a Clip,
) -> impl Iterator<Item = &'a Arc<Clip>> + 'a {
    seq.tracks()
        .flat_map(move |(_, t)| t.clips_in_range(clip.range))
        .filter(move |c| c.id != clip.id && c.link == Some(link) && c.audio.active().is_some())
}

/// The (file, container stream) pairs media clip `c` plays with selection
/// `sel`: an extracted item plays its stream of the original file (AU-004),
/// a file the streams `sel` picks. Empty for other clips.
fn played(library: &Library, c: &Clip, sel: Option<StreamSel>) -> Vec<(MediaId, u32)> {
    let ClipContent::Media { asset, .. } = c.content else {
        return Vec::new();
    };
    let Some(item) = library.get(asset) else {
        return Vec::new();
    };
    match item.source {
        MediaSource::AudioOf { source, stream } => vec![(source, stream)],
        MediaSource::File => item
            .info
            .played_streams(sel)
            .iter()
            .map(|s| (asset, s.index))
            .collect(),
    }
}

/// A media clip's stream selection (`None`: the media default).
fn selection(c: &Clip) -> Option<StreamSel> {
    match c.content {
        ClipContent::Media { stream, .. } => stream,
        _ => None,
    }
}

/// The first unlocked audio track free over `span` below every audio track
/// holding a clip of `link` (from the top when none does), else a new audio
/// track at the bottom of the audio band.
fn track_below_link(
    tx: &mut Tx<'_>,
    seq: SeqId,
    span: TimeRange,
    link: LinkId,
) -> Result<TrackId, EditError> {
    let tracks = tx.seq(seq)?.audio_tracks();
    let start = tracks
        .iter()
        .rposition(|t| t.clips().iter().any(|c| c.link == Some(link)))
        .map_or(0, |i| i + 1);
    let free = tracks
        .get(start..)
        .unwrap_or_default()
        .iter()
        .find(|t| !t.is_locked() && policy::fits(t, &[span], &BTreeSet::new()))
        .map(|t| t.id);
    if let Some(id) = free {
        return Ok(id);
    }
    let slot = TrackSlot::audio(tracks.len());
    let id = policy::add_track_at(tx, seq, TrackKind::Audio, slot)?;
    tx.notice(Notice::CreatedTrack(id));
    Ok(id)
}
