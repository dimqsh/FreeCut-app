//! Clip sound: detaching and restoring the sound of video clips (TL-030,
//! AU-003), the per-clip settings (AU-005, AU-006, AU-007) and the master
//! volume (AU-010).

use std::collections::BTreeSet;
use std::sync::Arc;

use super::{lane_order, resolve_clips};
use crate::edit::policy;
use crate::edit::tx::Tx;
use crate::edit::{Change, EditError, Label, Notice, OpKind, Plan};
use crate::ids::{ClipId, LinkId, SeqId, TrackId};
use crate::model::{
    Clip, ClipAudio, ClipContent, Document, EmbeddedAudio, FadeCurve, GainDb, MasterAudio,
    MediaKind, Sequence, StreamSel, TrackKind, TrackSlot,
};
use crate::time::{Tick, TimeRange};

/// One sound setting to put on clips.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioEdit {
    /// Static clip volume (AU-005). Clips whose volume has keyframes are
    /// skipped (their envelope is edited through
    /// [`crate::edit::EditRequest::EditGainKeys`]); refused when every clip
    /// with sound has keys.
    Gain(GainDb),
    /// Clip mute (AU-007).
    Muted(bool),
    /// Fade lengths (AU-006). Negative lengths are refused; lengths longer
    /// than the clip are stored as given and clamped when played.
    FadeIn(Tick),
    FadeOut(Tick),
    /// Fade shapes (AU-006, v1).
    FadeInCurve(FadeCurve),
    FadeOutCurve(FadeCurve),
}

impl AudioEdit {
    /// The settings with this edit applied; `None` when the edit doesn't
    /// apply to them (a static volume on a keyframed clip).
    fn applied_to(self, audio: &ClipAudio) -> Option<ClipAudio> {
        let audio = audio.clone();
        Some(match self {
            AudioEdit::Gain(gain) => {
                if audio.gain.is_animated() {
                    return None;
                }
                ClipAudio {
                    gain: audio.gain.with_static(gain),
                    ..audio
                }
            }
            AudioEdit::Muted(muted) => ClipAudio { muted, ..audio },
            AudioEdit::FadeIn(fade_in) => ClipAudio { fade_in, ..audio },
            AudioEdit::FadeOut(fade_out) => ClipAudio { fade_out, ..audio },
            AudioEdit::FadeInCurve(fade_in_curve) => ClipAudio {
                fade_in_curve,
                ..audio
            },
            AudioEdit::FadeOutCurve(fade_out_curve) => ClipAudio {
                fade_out_curve,
                ..audio
            },
        })
    }
}

/// A clip found by scanning a sequence.
type Found = (TrackSlot, TrackId, Arc<Clip>);

pub(crate) fn detach_audio(mut tx: Tx<'_>, seq: SeqId, ids: &[ClipId]) -> Result<Plan, EditError> {
    if ids.is_empty() {
        return Err(EditError::NothingSelected);
    }
    let mut eligible = Vec::new();
    for target in resolve_clips(&tx, seq, ids)? {
        let Some(audio) = target.clip.audio.active().cloned() else {
            continue;
        };
        let ClipContent::Media {
            asset,
            kind: MediaKind::Video,
            source,
            stream,
        } = target.clip.content
        else {
            continue;
        };
        let has_sound = tx
            .work()
            .library()
            .get(asset)
            .is_some_and(|m| m.info.has_audio());
        if has_sound {
            eligible.push((target, asset, source, stream, audio));
        }
    }
    if eligible.is_empty() {
        return Err(EditError::NothingToDetach);
    }
    let s = tx.seq(seq)?;
    for (target, ..) in &eligible {
        let track = s
            .track_at(target.loc.slot)
            .ok_or(EditError::StaleTrack(target.loc.track))?;
        policy::require_unlocked(track, target.loc.slot)?;
    }

    let mut created = Vec::with_capacity(eligible.len());
    for (target, asset, source, stream, audio) in eligible {
        let video = &target.clip;
        // A video already linked to streams added as clips (AU-038) keeps
        // its link, so those clips stay with its detached sound: the sync
        // badge, Restore and the "already playing" check still find them.
        let link = match video.link {
            Some(link) => link,
            None => tx.ids.link()?,
        };
        let id = tx.ids.clip()?;
        let mut sound = Clip::new(
            id,
            video.range,
            ClipContent::Media {
                asset,
                kind: MediaKind::Audio,
                source,
                // The detached clip plays what the video played (AU-038).
                stream,
            },
        );
        // The new clip sounds exactly as the embedded audio did (AU-003).
        sound.enabled = video.enabled;
        sound.name.clone_from(&video.name);
        sound.link = Some(link);
        sound.audio = EmbeddedAudio::Active(audio);
        let mut silent = (**video).clone();
        silent.audio = EmbeddedAudio::Detached(link);
        silent.link = Some(link);
        tx.push(Change::ReplaceClip {
            seq,
            track: target.loc.track,
            before: Arc::clone(video),
            after: Arc::new(silent),
        })?;
        let track = free_audio_track(&mut tx, seq, video.range)?;
        tx.push(Change::InsertClip {
            seq,
            track,
            clip: Arc::new(sound),
        })?;
        created.push(id);
    }
    let count = u32::try_from(created.len()).unwrap_or(u32::MAX);
    tx.select(created);
    tx.finish(Label {
        op: OpKind::DetachAudio,
        count,
    })
}

/// The first unlocked audio track, top-down, that is free over `span`, else
/// a new audio track at the top of the audio band (TL-030).
pub(super) fn free_audio_track(
    tx: &mut Tx<'_>,
    seq: SeqId,
    span: TimeRange,
) -> Result<TrackId, EditError> {
    let free = tx
        .seq(seq)?
        .audio_tracks()
        .iter()
        .find(|t| !t.is_locked() && policy::fits(t, &[span], &BTreeSet::new()))
        .map(|t| t.id);
    if let Some(id) = free {
        return Ok(id);
    }
    let id = policy::add_track_at(tx, seq, TrackKind::Audio, TrackSlot::audio(0))?;
    tx.notice(Notice::CreatedTrack(id));
    Ok(id)
}

/// The clips a restore of `clip` would remove: every clip in `seq` still
/// carrying the link of `clip`'s detached sound, earliest first (TL-030).
/// Empty when `clip` isn't a video clip in `seq` with detached sound. For the
/// app's confirmation; a scan of one sequence.
pub fn linked_audio(doc: &Document, seq: SeqId, clip: ClipId) -> Vec<ClipId> {
    let in_seq = doc.find_clip(clip).is_some_and(|loc| loc.seq == seq);
    let link = doc.clip(clip).and_then(|c| c.audio.detached());
    match (in_seq, link, doc.sequence(seq)) {
        (true, Some(link), Some(s)) => linked_sounds(s, link)
            .into_iter()
            .map(|(_, _, c)| c.id)
            .collect(),
        _ => Vec::new(),
    }
}

/// The audio clips carrying `link`, by start, then track order. Only audio
/// clips are the detached sound: anything else carrying the link (possible
/// only in a hand-edited file) is never taken for it, so a restore can't
/// delete it.
pub(super) fn linked_sounds(seq: &Sequence, link: LinkId) -> Vec<Found> {
    scan(seq, |c| {
        c.link == Some(link)
            && c.audio.detached() != Some(link)
            && matches!(
                c.content,
                ClipContent::Media {
                    kind: MediaKind::Audio,
                    ..
                }
            )
    })
}

/// The video clips whose sound is detached under `link`: one, or several if
/// it was split while detached. By start, then track order.
fn detached_videos(seq: &Sequence, link: LinkId) -> Vec<Found> {
    scan(seq, |c| c.audio.detached() == Some(link))
}

fn scan(seq: &Sequence, wanted: impl Fn(&Clip) -> bool) -> Vec<Found> {
    let mut found: Vec<Found> = seq
        .tracks()
        .flat_map(|(slot, t)| {
            t.clips()
                .iter()
                .filter(|c| wanted(c))
                .map(move |c| (slot, t.id, Arc::clone(c)))
        })
        .collect();
    found.sort_by_key(|(slot, _, c)| (c.start(), lane_order(*slot), c.id));
    found
}

/// The settings a restore gives back: the first linked clip's, with the
/// fade-out (and its curve) of the one ending last, so a detached clip that
/// was split restores both of its fades (AU-003). Defaults when none is
/// left.
///
/// Clips that read the video's own file are the detached sound; clips added
/// with "Add stream as clip" (AU-038) read an extracted item, and are
/// taken only when no detached piece is left.
fn restored_settings(sounds: &[Found], videos: &[Found]) -> ClipAudio {
    let active = |c: &Clip| c.audio.active().cloned().unwrap_or_default();
    let own: Vec<&Found> = sounds
        .iter()
        .filter(|(_, _, c)| {
            videos
                .iter()
                .any(|(_, _, v)| v.content.asset() == c.content.asset())
        })
        .collect();
    let pool: Vec<&Found> = if own.is_empty() {
        sounds.iter().collect()
    } else {
        own
    };
    let Some((_, _, first)) = pool.first() else {
        return ClipAudio::default();
    };
    let mut audio = active(first);
    if let Some((_, _, last)) = pool.iter().max_by_key(|(_, _, c)| c.end()) {
        let last = active(last);
        audio.fade_out = last.fade_out;
        audio.fade_out_curve = last.fade_out_curve;
    }
    audio
}

/// The stream selection a restore gives back to `video`: that of the first
/// linked clip reading the video's own file, so the video plays the stream
/// its detached sound played (AU-003, AU-038). `None` (the video keeps its
/// own) when no such clip is left.
fn restored_stream(sounds: &[Found], video: &Clip) -> Option<Option<StreamSel>> {
    let file = video.content.asset()?;
    sounds.iter().find_map(|(_, _, c)| match c.content {
        ClipContent::Media { asset, stream, .. } if asset == file => Some(stream),
        _ => None,
    })
}

pub(crate) fn restore_audio(mut tx: Tx<'_>, seq: SeqId, ids: &[ClipId]) -> Result<Plan, EditError> {
    if ids.is_empty() {
        return Err(EditError::NothingSelected);
    }
    let mut links: Vec<LinkId> = Vec::new();
    for target in resolve_clips(&tx, seq, ids)? {
        let video = matches!(
            target.clip.content,
            ClipContent::Media {
                kind: MediaKind::Video,
                ..
            }
        );
        if let (true, Some(link)) = (video, target.clip.audio.detached())
            && !links.contains(&link)
        {
            links.push(link);
        }
    }
    if links.is_empty() {
        return Err(EditError::NothingToRestore);
    }

    let s = tx.seq(seq)?;
    let mut work = Vec::with_capacity(links.len());
    for link in links {
        let (sounds, videos) = (linked_sounds(s, link), detached_videos(s, link));
        for (slot, id, _) in sounds.iter().chain(&videos) {
            let track = s.track_at(*slot).ok_or(EditError::StaleTrack(*id))?;
            policy::require_unlocked(track, *slot)?;
        }
        work.push((sounds, videos));
    }

    let mut restored = Vec::new();
    for (sounds, videos) in work {
        let settings = restored_settings(&sounds, &videos);
        let streams: Vec<_> = videos
            .iter()
            .map(|(_, _, v)| restored_stream(&sounds, v))
            .collect();
        for (_, track, clip) in sounds {
            tx.push(Change::RemoveClip { seq, track, clip })?;
        }
        // Every piece of a video split while detached gets its sound back;
        // inner edges have no fade, as after a split (AU-006).
        let last = videos.len().saturating_sub(1);
        for (i, ((_, track, before), sel)) in videos.into_iter().zip(streams).enumerate() {
            let mut audio = settings.clone();
            if i > 0 {
                audio = audio.right_of_split();
            }
            if i < last {
                audio = audio.left_of_split();
            }
            let mut after = (*before).clone();
            after.audio = EmbeddedAudio::Active(audio);
            after.link = None;
            if let (Some(sel), ClipContent::Media { stream, .. }) = (sel, &mut after.content) {
                *stream = sel;
            }
            restored.push(before.id);
            tx.push(Change::ReplaceClip {
                seq,
                track,
                before,
                after: Arc::new(after),
            })?;
        }
    }
    let count = u32::try_from(restored.len()).unwrap_or(u32::MAX);
    tx.select(restored);
    tx.finish(Label {
        op: OpKind::RestoreAudio,
        count,
    })
}

pub(crate) fn set_clip_audio(
    mut tx: Tx<'_>,
    seq: SeqId,
    ids: &[ClipId],
    edit: AudioEdit,
) -> Result<Plan, EditError> {
    if ids.is_empty() {
        return Err(EditError::NothingSelected);
    }
    if let AudioEdit::FadeIn(len) | AudioEdit::FadeOut(len) = edit
        && len.is_negative()
    {
        return Err(EditError::NegativeFade);
    }
    let with_sound: Vec<_> = resolve_clips(&tx, seq, ids)?
        .into_iter()
        .filter_map(|r| r.clip.audio.active().cloned().map(|a| (r, a)))
        .collect();
    if with_sound.is_empty() {
        return Err(EditError::NoClipAudio);
    }
    let targets: Vec<_> = with_sound
        .into_iter()
        .filter_map(|(r, before)| edit.applied_to(&before).map(|after| (r, before, after)))
        .collect();
    if targets.is_empty() {
        return Err(EditError::VolumeKeyframed);
    }
    let s = tx.seq(seq)?;
    for (target, ..) in &targets {
        let track = s
            .track_at(target.loc.slot)
            .ok_or(EditError::StaleTrack(target.loc.track))?;
        policy::require_unlocked(track, target.loc.slot)?;
    }
    let mut count: u32 = 0;
    for (target, before, after) in targets {
        if after == before {
            continue;
        }
        let mut clip = (*target.clip).clone();
        clip.audio = EmbeddedAudio::Active(after);
        tx.push(Change::ReplaceClip {
            seq,
            track: target.loc.track,
            before: target.clip,
            after: Arc::new(clip),
        })?;
        count = count.saturating_add(1);
    }
    tx.finish(Label {
        op: OpKind::SetClipAudio,
        count,
    })
}

/// Sets the master volume of `seq` (AU-010), clamped to the master range.
pub(crate) fn set_master_gain(mut tx: Tx<'_>, seq: SeqId, gain: GainDb) -> Result<Plan, EditError> {
    let before = tx.seq(seq)?.master;
    let after = MasterAudio {
        gain: MasterAudio::clamp_gain(gain),
    };
    let count = u32::from(after != before);
    if after != before {
        tx.push(Change::SetMaster { seq, before, after })?;
    }
    tx.finish(Label {
        op: OpKind::SetMasterGain,
        count,
    })
}
