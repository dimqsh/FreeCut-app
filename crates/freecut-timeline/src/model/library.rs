//! The project's media library as plain data (PM-030, PM-035, PM-038,
//! PM-100). `freecut-media` fills these types by probing files; nothing here
//! touches the file system.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::clip::StreamSel;
use crate::ids::MediaId;
use crate::time::{Rational, Tick};

/// Where a media file lives, as stored in the project (PM-100). Both forms
/// are kept so a moved project folder still finds its media.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StoredPath {
    /// The absolute path when last resolved, in the OS's native form.
    pub absolute: String,
    /// The path relative to the project file, when it has one (`/`
    /// separators).
    pub relative: Option<String>,
}

/// Identifies a file's content without reading all of it (PM-038).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fingerprint {
    pub size: u64,
    /// xxh3 of the first and last 1 MiB (the whole file when smaller).
    pub head_tail: u64,
    pub duration: Tick,
}

/// How a picture is rotated for display (container metadata).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Rotation {
    #[default]
    None,
    Cw90,
    Cw180,
    Cw270,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoStreamInfo {
    /// Container stream index.
    pub index: u32,
    pub codec: String,
    /// Coded size, before rotation.
    pub width: u32,
    pub height: u32,
    /// Sample aspect ratio (1:1 for square pixels).
    pub sar: (u32, u32),
    pub rotation: Rotation,
    pub mirrored: bool,
    /// Nominal frame rate, exact. Source rates need not be on the tick grid.
    pub rate: Option<Rational>,
    pub variable_rate: bool,
    pub has_alpha: bool,
    pub bit_depth: u8,
    /// First presentable time relative to the media origin.
    pub start: Tick,
    pub duration: Tick,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioStreamInfo {
    pub index: u32,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u16,
    /// May be negative: audio that starts before the first video frame.
    pub start: Tick,
    pub duration: Tick,
    /// False when the length was estimated rather than measured.
    pub duration_exact: bool,
    /// The file flags this stream as the one to play by default (AU-038).
    pub default: bool,
    /// Language tag from the file, as stored there (for example "eng").
    pub language: Option<String>,
    /// Stream title from the file.
    pub title: Option<String>,
    /// Channel layout name, as the probe reports it (for example "stereo").
    pub layout: Option<String>,
}

/// What probing found (PM-035). Times are media time: 0 is the first
/// presentable frame of the default video stream (or of the first audio
/// stream when there is no video).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaInfo {
    pub container: String,
    /// Container index of the default video stream, which defines media
    /// time 0.
    pub default_video: Option<u32>,
    pub duration: Tick,
    pub video: Vec<VideoStreamInfo>,
    pub audio: Vec<AudioStreamInfo>,
    /// Set for still images (and image sequences, later): their pixel size.
    pub still: Option<(u32, u32)>,
}

impl MediaInfo {
    pub fn is_still(&self) -> bool {
        self.still.is_some()
    }

    /// The audio stream with container index `index`.
    pub fn audio_stream(&self, index: u32) -> Option<&AudioStreamInfo> {
        self.audio.iter().find(|a| a.index == index)
    }

    /// The audio stream played by default (AU-038, PM-040): the one the
    /// file flags as default, else the first.
    pub fn default_audio(&self) -> Option<&AudioStreamInfo> {
        self.audio
            .iter()
            .find(|a| a.default)
            .or_else(|| self.audio.first())
    }

    /// The audio streams a clip with stream selection `sel` plays (AU-038):
    /// the chosen stream, every stream for [`StreamSel::AllMixed`], and the
    /// default stream for `None` or for a chosen stream the file no longer
    /// has (after a relink). Empty without sound.
    pub fn played_streams(&self, sel: Option<StreamSel>) -> &[AudioStreamInfo] {
        let one = |i: usize| &self.audio[i..=i];
        let index_of = |stream: u32| self.audio.iter().position(|a| a.index == stream);
        let default = || {
            self.audio
                .iter()
                .position(|a| a.default)
                .or_else(|| (!self.audio.is_empty()).then_some(0))
        };
        match sel {
            Some(StreamSel::AllMixed) => &self.audio,
            Some(StreamSel::Index(stream)) => {
                index_of(stream).or_else(default).map_or(&[][..], one)
            }
            None => default().map_or(&[][..], one),
        }
    }

    /// What an [`MediaSource::AudioOf`] item of this media sees: only the
    /// audio stream `stream`, on the same media timeline (same duration,
    /// same stream start). `None` if there is no such stream.
    pub fn audio_only(&self, stream: u32) -> Option<MediaInfo> {
        let audio = self.audio_stream(stream)?.clone();
        Some(MediaInfo {
            container: self.container.clone(),
            default_video: None,
            duration: self.duration,
            video: Vec::new(),
            audio: vec![audio],
            still: None,
        })
    }

    pub fn has_video(&self) -> bool {
        !self.video.is_empty()
    }

    pub fn has_audio(&self) -> bool {
        !self.audio.is_empty()
    }
}

/// What a media item reads (PM-041).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum MediaSource {
    /// An imported file, read whole.
    #[default]
    File,
    /// A virtual audio-only item: audio stream `stream` (container index) of
    /// the file of item `source` (AU-004). The item carries its own copy of
    /// that file's path and fingerprint, so it keeps working if `source` is
    /// removed. Its media time is the source file's, so source ranges line
    /// up with clips of the original video.
    AudioOf { source: MediaId, stream: u32 },
}

/// What kind of file FreeCut produced for a generated item (PM-041).
/// Producing areas add variants as they land.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GeneratedMediaKind {
    /// A voiceover take recorded in the app (AU-027).
    Voiceover,
    /// Text-to-speech audio (AI-025).
    Tts,
}

/// Where a media item came from (PM-041).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MediaOrigin {
    /// A file the user imported.
    #[default]
    Imported,
    /// A file FreeCut made: a recording, generated speech and so on.
    Generated { kind: GeneratedMediaKind },
}

/// One item in the media library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaItem {
    pub id: MediaId,
    /// What the item reads: a file, or part of another item's file.
    pub source: MediaSource,
    /// Imported or generated (PM-041).
    pub origin: MediaOrigin,
    pub path: StoredPath,
    /// The last known probe result; kept while the file is offline.
    pub info: MediaInfo,
    pub fingerprint: Option<Fingerprint>,
    /// A user-chosen name; `None` shows the file name.
    pub display_name: Option<String>,
}

/// The project's media items, by id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Library {
    items: BTreeMap<MediaId, Arc<MediaItem>>,
}

impl Library {
    pub fn from_parts(items: Vec<Arc<MediaItem>>) -> Result<Library, MediaId> {
        let mut map = BTreeMap::new();
        for item in items {
            let id = item.id;
            if map.insert(id, item).is_some() {
                return Err(id);
            }
        }
        Ok(Library { items: map })
    }

    pub fn get(&self, id: MediaId) -> Option<&Arc<MediaItem>> {
        self.items.get(&id)
    }

    pub fn items(&self) -> impl Iterator<Item = &Arc<MediaItem>> {
        self.items.values()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The media's last known length.
    pub fn duration(&self, id: MediaId) -> Option<Tick> {
        self.items.get(&id).map(|m| m.info.duration)
    }

    /// The items extracted from `parent`'s audio (AU-004), in id order.
    pub fn audio_items_of(&self, parent: MediaId) -> impl Iterator<Item = &Arc<MediaItem>> {
        self.items.values().filter(
            move |m| matches!(m.source, MediaSource::AudioOf { source, .. } if source == parent),
        )
    }

    pub(crate) fn insert(&mut self, item: Arc<MediaItem>) -> bool {
        match self.items.entry(item.id) {
            std::collections::btree_map::Entry::Occupied(_) => false,
            std::collections::btree_map::Entry::Vacant(v) => {
                v.insert(item);
                true
            }
        }
    }

    pub(crate) fn remove(&mut self, id: MediaId) -> Option<Arc<MediaItem>> {
        self.items.remove(&id)
    }

    pub(crate) fn replace(&mut self, item: Arc<MediaItem>) -> Option<Arc<MediaItem>> {
        self.items.insert(item.id, item)
    }
}
