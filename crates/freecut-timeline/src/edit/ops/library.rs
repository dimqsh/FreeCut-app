//! Media library and project settings edits (PM-005, PM-011, PM-030,
//! PM-041, PM-058, PM-091, AU-004). Probing and file access happen in
//! `freecut-media` and `freecut-app`; these planners only record the
//! results.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::conform::conform_in;
use super::delete::{DeleteReq, delete_in};
use crate::edit::tx::Tx;
use crate::edit::{Change, EditError, Label, Notice, OpKind, Plan};
use crate::ids::{ClipId, MediaId, SeqId};
use crate::model::{
    ClipAudio, ClipContent, EmbeddedAudio, Fingerprint, MediaInfo, MediaItem, MediaKind,
    MediaOrigin, MediaSource, ProjectSettings, StoredPath,
};

/// A probed file to add to the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMedia {
    pub path: StoredPath,
    pub info: MediaInfo,
    pub fingerprint: Option<Fingerprint>,
    pub display_name: Option<String>,
}

/// Adds probed files to the library (PM-030). An item with more than one
/// audio stream is announced ([`Notice::MultipleAudioStreams`], AU-038).
pub(crate) fn import_media(mut tx: Tx<'_>, items: &[NewMedia]) -> Result<Plan, EditError> {
    for new in items {
        let id = tx.ids.media()?;
        if new.info.audio.len() > 1
            && let Some(default) = new.info.default_audio()
        {
            tx.notice(Notice::MultipleAudioStreams {
                media: id,
                count: u32::try_from(new.info.audio.len()).unwrap_or(u32::MAX),
                default_stream: default.index,
            });
        }
        tx.push(Change::InsertMedia {
            item: Arc::new(MediaItem {
                id,
                source: MediaSource::File,
                origin: MediaOrigin::Imported,
                path: new.path.clone(),
                info: new.info.clone(),
                fingerprint: new.fingerprint,
                display_name: new.display_name.clone(),
            }),
        })?;
        tx.created_media(id);
    }
    tx.finish(Label {
        op: OpKind::ImportMedia,
        count: u32::try_from(items.len()).unwrap_or(u32::MAX),
    })
}

/// Removes media from the library. Media still used on the timeline is
/// refused unless `with_clips`, which deletes those clips in the same step
/// (PM-058).
pub(crate) fn remove_media(
    mut tx: Tx<'_>,
    media: &[MediaId],
    with_clips: bool,
) -> Result<Plan, EditError> {
    if !with_clips
        && let Some(used) = media
            .iter()
            .copied()
            .find(|m| !clips_using(&tx, &[*m]).is_empty())
    {
        return Err(EditError::MediaInUse(used));
    }
    for (seq, clips) in clips_using(&tx, media) {
        delete_in(
            &mut tx,
            &DeleteReq {
                seq,
                clips,
                ripple: false,
            },
        )?;
    }
    let mut count = 0;
    for id in media {
        let item = tx
            .work()
            .library()
            .get(*id)
            .cloned()
            .ok_or(EditError::UnknownMedia(*id))?;
        tx.push(Change::RemoveMedia { item })?;
        count += 1;
    }
    tx.finish(Label {
        op: OpKind::RemoveMedia,
        count,
    })
}

/// Points a media item at a different file (PM-091) or records a corrected
/// probe result (PM-093, exact length). Refused if a clip would then read
/// past the end of the media.
///
/// Items extracted from the file (AU-004) follow it in the same step: they
/// take the new path and fingerprint and see their stream of the new info.
/// A stream is followed by its place among the file's audio streams, so a
/// file whose stream indices shifted (a `.mov` with a timecode track) still
/// matches. One whose stream the new file lacks is left as it was, to be
/// relinked on its own, and the plan says so
/// ([`Notice::ExtractedAudioNotRelinked`]).
///
/// An extracted item updated directly keeps only its own stream of `info`;
/// when the new file has no stream with that index, it takes the first audio
/// stream no other item extracted from the same original plays.
///
/// Video clips of a relinked item gain or lose their embedded sound with the
/// file (AU-002): none becomes active (default settings) when the new file
/// has sound, active becomes none when it has none. Detached sound stays.
pub(crate) fn update_media(
    mut tx: Tx<'_>,
    media: MediaId,
    path: Option<&StoredPath>,
    info: &MediaInfo,
    fingerprint: Option<Fingerprint>,
) -> Result<Plan, EditError> {
    let before = tx
        .work()
        .library()
        .get(media)
        .cloned()
        .ok_or(EditError::UnknownMedia(media))?;
    match before.source {
        MediaSource::File => {
            replace_media(
                &mut tx,
                &before,
                before.source,
                path,
                info.clone(),
                fingerprint,
            )?;
            let extracted: Vec<_> = tx.work().library().audio_items_of(media).cloned().collect();
            for item in extracted {
                let MediaSource::AudioOf { source, stream } = item.source else {
                    continue;
                };
                let followed = followed_stream(&before.info, info, stream)
                    .and_then(|s| Some((s, info.audio_only(s)?)));
                match followed {
                    Some((stream, own_info)) => {
                        let source = MediaSource::AudioOf { source, stream };
                        replace_media(&mut tx, &item, source, path, own_info, fingerprint)?;
                    }
                    None => tx.notice(Notice::ExtractedAudioNotRelinked(item.id)),
                }
            }
        }
        MediaSource::AudioOf { source, stream } => {
            let stream = own_stream(&tx, &before, source, stream, info)?;
            let own_info = info
                .audio_only(stream)
                .ok_or(EditError::UnknownStream { media, stream })?;
            let source = MediaSource::AudioOf { source, stream };
            replace_media(&mut tx, &before, source, path, own_info, fingerprint)?;
        }
    }
    let op = if path.is_some() {
        OpKind::RelinkMedia
    } else {
        OpKind::UpdateMediaInfo
    };
    tx.finish(Label { op, count: 1 })
}

/// The stream of `new` that an item extracted from stream `stream` of a file
/// probed as `old` plays after the file is replaced: the one at the same
/// place among the audio streams, else the one with the same index.
fn followed_stream(old: &MediaInfo, new: &MediaInfo, stream: u32) -> Option<u32> {
    old.audio
        .iter()
        .position(|a| a.index == stream)
        .and_then(|place| new.audio.get(place))
        .map(|a| a.index)
        .or_else(|| new.audio_stream(stream).map(|a| a.index))
}

/// The stream an extracted item relinked on its own plays in `info`: its own
/// index if the file has it, else the first audio stream no other item
/// extracted from `parent` plays.
fn own_stream(
    tx: &Tx<'_>,
    item: &MediaItem,
    parent: MediaId,
    stream: u32,
    info: &MediaInfo,
) -> Result<u32, EditError> {
    if info.audio_stream(stream).is_some() {
        return Ok(stream);
    }
    if !info.has_audio() {
        return Err(EditError::NoSound(item.id));
    }
    let taken: Vec<u32> = tx
        .work()
        .library()
        .audio_items_of(parent)
        .filter(|m| m.id != item.id)
        .filter_map(|m| match m.source {
            MediaSource::AudioOf { stream, .. } => Some(stream),
            MediaSource::File => None,
        })
        .collect();
    info.audio
        .iter()
        .map(|a| a.index)
        .find(|s| !taken.contains(s))
        .ok_or(EditError::UnknownStream {
            media: item.id,
            stream,
        })
}

/// Records new file data for one item, after checking every clip using it
/// still fits in the media, and brings the embedded sound of its video clips
/// in line with whether the new file has sound (AU-002).
fn replace_media(
    tx: &mut Tx<'_>,
    before: &Arc<MediaItem>,
    source: MediaSource,
    path: Option<&StoredPath>,
    info: MediaInfo,
    fingerprint: Option<Fingerprint>,
) -> Result<(), EditError> {
    let users = clips_using(tx, &[before.id]);
    for clips in users.values() {
        for id in clips {
            let clip = tx.clip(*id)?;
            if clip
                .content
                .source()
                .is_some_and(|s| s.end() > info.duration)
            {
                return Err(EditError::SourceLimit { clip: *id });
            }
        }
    }
    let sound = info.has_audio();
    let mut after = (**before).clone();
    after.source = source;
    if let Some(path) = path {
        after.path = path.clone();
    }
    after.info = info;
    after.fingerprint = fingerprint.or(before.fingerprint);
    if after != **before {
        tx.push(Change::ReplaceMedia {
            before: Arc::clone(before),
            after: Arc::new(after),
        })?;
    }
    if sound == before.info.has_audio() {
        return Ok(());
    }
    for (seq, clips) in users {
        for id in clips {
            let clip = tx.clip(id)?;
            let video = matches!(
                clip.content,
                ClipContent::Media {
                    kind: MediaKind::Video,
                    ..
                }
            );
            let audio = match clip.audio {
                EmbeddedAudio::None if sound => EmbeddedAudio::Active(ClipAudio::default()),
                EmbeddedAudio::Active(_) if !sound => EmbeddedAudio::None,
                _ => continue,
            };
            if !video {
                continue;
            }
            let loc = tx.locate(id)?;
            let mut changed = (*clip).clone();
            changed.audio = audio;
            tx.push(Change::ReplaceClip {
                seq,
                track: loc.track,
                before: clip,
                after: Arc::new(changed),
            })?;
        }
    }
    Ok(())
}

/// Adds a virtual library item playing one audio stream of `media` (AU-004,
/// PM-041), named after the original. Extracting a stream that already has
/// an item reuses that item: nothing changes, and the plan reports it as
/// imported so the app can select it. An extracted item is its own extract.
pub(crate) fn extract_audio(
    mut tx: Tx<'_>,
    media: MediaId,
    stream: Option<u32>,
) -> Result<Plan, EditError> {
    let label = Label {
        op: OpKind::ExtractAudio,
        count: 1,
    };
    let parent = tx
        .work()
        .library()
        .get(media)
        .cloned()
        .ok_or(EditError::UnknownMedia(media))?;
    if matches!(parent.source, MediaSource::AudioOf { .. }) {
        tx.created_media(media);
        return tx.finish(Label { count: 0, ..label });
    }
    let stream = match stream {
        Some(stream) => stream,
        None => {
            parent
                .info
                .default_audio()
                .ok_or(EditError::NoSound(media))?
                .index
        }
    };
    let (id, created) = audio_item(&mut tx, &parent, stream)?;
    tx.created_media(id);
    tx.finish(Label {
        count: u32::from(created),
        ..label
    })
}

/// The library item playing audio stream `stream` of `parent` (AU-004,
/// PM-041): the existing one, or a new one added now (`true`). The parent
/// must be a file item.
pub(super) fn audio_item(
    tx: &mut Tx<'_>,
    parent: &MediaItem,
    stream: u32,
) -> Result<(MediaId, bool), EditError> {
    let media = parent.id;
    let info = parent
        .info
        .audio_only(stream)
        .ok_or(if parent.info.has_audio() {
            EditError::UnknownStream { media, stream }
        } else {
            EditError::NoSound(media)
        })?;
    let source = MediaSource::AudioOf {
        source: media,
        stream,
    };
    let existing = tx
        .work()
        .library()
        .audio_items_of(media)
        .find(|m| m.source == source)
        .map(|m| m.id);
    if let Some(existing) = existing {
        return Ok((existing, false));
    }
    let id = tx.ids.media()?;
    tx.push(Change::InsertMedia {
        item: Arc::new(MediaItem {
            id,
            source,
            origin: MediaOrigin::Imported,
            path: parent.path.clone(),
            info,
            fingerprint: parent.fingerprint,
            display_name: Some(format!("{} (audio)", shown_name(parent))),
        }),
    })?;
    Ok((id, true))
}

/// The name the library shows for an item: its display name, else the file
/// name.
fn shown_name(item: &MediaItem) -> &str {
    item.display_name.as_deref().unwrap_or_else(|| {
        let path = item.path.absolute.as_str();
        path.rsplit(['/', '\\'])
            .next()
            .filter(|name| !name.is_empty())
            .unwrap_or(path)
    })
}

/// Changes the project settings (PM-004, PM-005). With `conform`, every
/// sequence is conformed to the new frame grid in the same step (PM-011).
pub(crate) fn set_project_settings(
    mut tx: Tx<'_>,
    settings: &ProjectSettings,
    conform: bool,
) -> Result<Plan, EditError> {
    if !settings.canvas_is_valid() {
        return Err(EditError::InvalidCanvas);
    }
    let before = *tx.work().settings();
    if before != *settings {
        tx.push(Change::SetSettings {
            before,
            after: *settings,
        })?;
    }
    if conform {
        let seqs: Vec<SeqId> = tx.work().sequences().map(|s| s.id).collect();
        for seq in seqs {
            conform_in(&mut tx, seq)?;
        }
    }
    tx.finish(Label {
        op: OpKind::SetProjectSettings,
        count: 1,
    })
}

/// Clips playing any of `media`, by sequence.
fn clips_using(tx: &Tx<'_>, media: &[MediaId]) -> BTreeMap<SeqId, Vec<ClipId>> {
    let mut out: BTreeMap<SeqId, Vec<ClipId>> = BTreeMap::new();
    for seq in tx.work().sequences() {
        for (_, track) in seq.tracks() {
            for clip in track.clips() {
                if clip.content.asset().is_some_and(|a| media.contains(&a)) {
                    out.entry(seq.id).or_default().push(clip.id);
                }
            }
        }
    }
    out
}
