//! Mapping between the format's DTOs and the timeline model.
//!
//! Reading checks every value through the model's own constructors
//! (`FrameRate::new`, `TimeRange::new`, …) and reports a refusal with the
//! JSON path of the offending value; nothing is rounded or guessed. Unknown
//! keys are moved into [`Extras`] on the way in and put back on the way out.
//!
//! The reader stops at "parts" (settings, media items, sequence pieces):
//! repairs and whole-document validation happen in [`crate::open`]. The
//! exceptions, both only ever needed for a hand-edited file: transform,
//! text, keyframe and animation values outside their ranges (TR-001, TX-004
//! to TX-016, TX-055, FX-018) are clamped here, the way an edit would clamp
//! them ([`text`] and [`motion`] have the rules), and volume and property
//! keyframes out of time order are sorted (AU-008, TR-027); both are
//! counted so the open reports a repair.
//! Audio values out of range (gains, pans) are refused, like clip gains
//! always were.
//!
//! Every number the file holds must survive any JSON reader exactly, so ids
//! stay at or below [`MAX_ID`], the id counter at or below `MAX_ID + 1`, and
//! ticks within ±[`MAX_TICK`]. The reader refuses values outside these
//! bounds and [`check_writable`] refuses to write them.

use core::fmt::Display;
use core::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use freecut_timeline::ids::{ClipId, GroupId, LinkId, MAX_ID, MarkerId, MediaId, SeqId, TrackId};
use freecut_timeline::model::{
    Angle, Animatable, Attachment, AudioStreamInfo, CanvasPos, Clip, ClipAnimations, ClipAudio,
    ClipContent, ClipKeys, ColorStack, Document, EmbeddedAudio, FadeCurve, Fingerprint, FollowKind,
    GainDb, GeneratedKind, GeneratedMediaKind, Keyframe, MAX_MARKER_NAME_CHARS,
    MAX_MARKER_NOTE_CHARS, Marker, MarkerOrigin, Markers, MasterAudio, MediaInfo, MediaItem,
    MediaKind, MediaOrigin, MediaSource, PaletteColor, Pan, PerMille, ProjectSettings, Rgba8,
    Rotation, Scale, Sequence, SequenceFlags, StoredPath, StreamSel, TextOrigin, Track, TrackAudio,
    TrackFlags, TrackHeader, TrackKind, Transform, VideoStreamInfo,
};
use freecut_timeline::time::{FrameRate, MarkRange, Rational, SampleRate, Tick, TimeRange};
use serde::de::DeserializeOwned;
use serde_json::Value;
use serde_path_to_error::Segment;

use super::dto::*;
use super::{APP_ID, CURRENT_FORMAT_VERSION, FORMAT_MAGIC};
use crate::export::{
    AudioExportSettings, Container, ExportSettings, FileTarget, FrameRateChoice, OutputSize,
    QualityTier, ShortSide, VideoCodec, VideoSettings,
};
use crate::extras::{Extras, Fields, Owner};
use crate::open::OpenError;
use crate::paths::rebase_relative;
use crate::ui::UiState;

mod color;
pub(crate) mod motion;
mod text;

use text::{read_text, write_text};

/// The largest tick magnitude a file holds: 2^53, about 148 days, so every
/// JSON reader parses ticks exactly (project-format value encoding rules).
pub(crate) const MAX_TICK: u64 = 1 << 53;

/// The largest id counter a file holds: one past [`MAX_ID`], the position of
/// an allocator that has handed out every id.
pub(crate) const MAX_IDS_NEXT: u64 = MAX_ID + 1;

/// A JSON path in the same notation as `serde_path_to_error`
/// (`document.sequences[0].visual[1].clips[2].range`).
#[derive(Debug, Clone)]
struct At(String);

impl At {
    fn root() -> At {
        At(String::new())
    }

    fn key(&self, key: &str) -> At {
        if self.0.is_empty() {
            At(key.to_string())
        } else {
            At(format!("{}.{key}", self.0))
        }
    }

    fn index(&self, i: usize) -> At {
        At(format!("{}[{i}]", self.0))
    }

    /// This path followed by a path inside the value at it.
    fn descend(&self, inner: &serde_path_to_error::Path) -> At {
        inner
            .iter()
            .fold(self.clone(), |at, segment| match segment {
                Segment::Seq { index } => at.index(*index),
                other => at.key(&other.to_string()),
            })
    }

    /// The path as error reports spell it: `.` for the whole file.
    fn spelled(&self) -> String {
        if self.0.is_empty() {
            ".".to_string()
        } else {
            self.0.clone()
        }
    }

    fn bad(&self, message: impl Display) -> OpenError {
        OpenError::BadValue {
            path: self.spelled(),
            message: message.to_string(),
        }
    }

    /// A schema error (wrong type, missing key) at this path.
    fn schema(&self, message: impl Display) -> OpenError {
        OpenError::Json {
            path: self.spelled(),
            message: message.to_string(),
        }
    }
}

/// A colour stack outside a project (a saved look, CL-026): the JSON at
/// `path` read with the clip rules of [`color`] (defaults left out, values
/// out of range clamped, a list that is no curve refused at its path).
/// Returns the stack and whether anything was clamped. Unknown keys are
/// dropped.
pub(crate) fn read_stack(value: Value, path: &str) -> Result<(ColorStack, bool), OpenError> {
    let at = At::root().key(path);
    let dto: ColorDto = decode_at(value, &at)?;
    let mut extras = Extras::new();
    let mut clamped = false;
    let adjustment = ClipContent::Generated {
        kind: GeneratedKind::Adjustment,
    };
    let stack = color::read_color(
        dto,
        &at,
        &adjustment,
        Owner::Root,
        &mut extras,
        &mut clamped,
    )?
    .unwrap_or_default();
    Ok((stack, clamped))
}

/// `stack` as the JSON [`read_stack`] reads, every default left out
/// (`{}` for the default stack).
pub(crate) fn write_stack(stack: &ColorStack) -> Value {
    let extras = Extras::new();
    let give = Give::new(&extras, Owner::Root);
    color::write_color(Some(stack), &give)
        .and_then(|dto| serde_json::to_value(dto).ok())
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()))
}

/// Decodes `value`, the value at `at`, as a `T`; errors name the full path.
fn decode_at<T: DeserializeOwned>(value: Value, at: &At) -> Result<T, OpenError> {
    serde_path_to_error::deserialize(value).map_err(|e| at.descend(e.path()).schema(e.into_inner()))
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// A sequence as read, before the main-track repair.
#[derive(Debug)]
pub(crate) struct SeqParts {
    pub id: SeqId,
    pub flags: SequenceFlags,
    pub master: MasterAudio,
    pub markers: Markers,
    pub visual: Vec<Arc<Track>>,
    pub audio: Vec<Arc<Track>>,
}

impl SeqParts {
    pub fn into_sequence(self) -> Arc<Sequence> {
        let mut seq = Sequence::from_parts(self.id, self.flags, self.visual, self.audio);
        seq.master = self.master;
        seq.markers = self.markers;
        Arc::new(seq)
    }
}

/// Everything read from a file, before repair and validation.
#[derive(Debug)]
pub(crate) struct Parts {
    pub app: String,
    pub ids_next: NonZeroU64,
    pub root: SeqId,
    pub settings: ProjectSettings,
    pub library: Vec<Arc<MediaItem>>,
    pub sequences: Vec<SeqParts>,
    pub export: Option<ExportSettings>,
    /// As stored: ids that are no sound-carrying track are dropped once the
    /// document is built.
    pub ui: UiState,
    pub extras: Extras,
    /// Clips whose transform or text values were clamped into range.
    pub clamped_clips: usize,
    /// Clips whose volume keyframes were out of time order and were sorted.
    pub sorted_clips: usize,
    /// Marker lists (a sequence's or a clip's) that were out of order or
    /// held a name or note over its length limit, and were fixed.
    pub fixed_marker_lists: usize,
}

/// State shared by the clip readers.
struct ClipReader<'a> {
    extras: &'a mut Extras,
    clamped: usize,
    sorted: usize,
    /// Marker lists fixed so far ([`Parts::fixed_marker_lists`]).
    markers: usize,
}

pub(crate) fn read(env: EnvelopeDto) -> Result<Parts, OpenError> {
    if env.ids_next.get() > MAX_IDS_NEXT {
        return Err(At::root().key("ids_next").bad(format!(
            "the id counter {} is above the largest {MAX_IDS_NEXT}",
            env.ids_next
        )));
    }
    let mut extras = Extras::new();
    extras.put(Owner::Root, "", env.extra);
    let doc = env.document;
    let at = At::root().key("document");
    extras.put(Owner::Document, "", doc.extra);
    let root = SeqId::from_raw(id(doc.root, &at.key("root"))?);
    let settings = read_settings(doc.settings, &at.key("settings"), &mut extras)?;
    let library = doc
        .library
        .into_iter()
        .enumerate()
        .map(|(i, m)| read_media(m, &at.key("library").index(i), &mut extras))
        .collect::<Result<Vec<_>, _>>()?;
    let mut reader = ClipReader {
        extras: &mut extras,
        clamped: 0,
        sorted: 0,
        markers: 0,
    };
    let sequences = doc
        .sequences
        .into_iter()
        .enumerate()
        .map(|(i, s)| read_sequence(s, &at.key("sequences").index(i), &mut reader))
        .collect::<Result<Vec<_>, _>>()?;
    let (clamped_clips, sorted_clips, fixed_marker_lists) =
        (reader.clamped, reader.sorted, reader.markers);
    let export = match env.export {
        Some(dto) => Some(read_export(dto, &At::root().key("export"), &mut extras)?),
        None => None,
    };
    let ui = match env.ui {
        Some(dto) => read_ui(dto, &At::root().key("ui"), &mut extras)?,
        None => UiState::default(),
    };
    Ok(Parts {
        app: env.app,
        ids_next: env.ids_next,
        root,
        settings,
        library,
        sequences,
        export,
        ui,
        extras,
        clamped_clips,
        sorted_clips,
        fixed_marker_lists,
    })
}

/// The UI state as stored; ids must be valid ids, whatever they name.
fn read_ui(dto: UiDto, at: &At, extras: &mut Extras) -> Result<UiState, OpenError> {
    extras.put(Owner::Ui, "", dto.extra);
    let solo = dto
        .solo
        .into_iter()
        .enumerate()
        .map(|(i, raw)| id(raw, &at.key("solo").index(i)).map(TrackId::from_raw))
        .collect::<Result<_, _>>()?;
    Ok(UiState { solo })
}

/// Ids are non-zero (serde checks that) and at most 2^53 so any JSON reader
/// parses them exactly.
fn id(raw: NonZeroU64, at: &At) -> Result<NonZeroU64, OpenError> {
    if raw.get() > MAX_ID {
        return Err(at.bad(format!("id {raw} is above the largest id {MAX_ID}")));
    }
    Ok(raw)
}

/// A tick value; at most [`MAX_TICK`] from zero so any JSON reader parses it
/// exactly.
fn tick(raw: i64, at: &At) -> Result<Tick, OpenError> {
    if raw.unsigned_abs() > MAX_TICK {
        return Err(at.bad(format!(
            "{raw} ticks is outside the supported range of ±{MAX_TICK}"
        )));
    }
    Ok(Tick::from_raw(raw))
}

fn range(dto: &RangeDto, at: &At) -> Result<TimeRange, OpenError> {
    let start = tick(dto.start, &at.key("start"))?;
    let duration = tick(dto.duration, &at.key("duration"))?;
    TimeRange::new(start, duration).map_err(|e| at.bad(e))
}

fn read_settings(
    dto: SettingsDto,
    at: &At,
    extras: &mut Extras,
) -> Result<ProjectSettings, OpenError> {
    let frame_rate = FrameRate::new(dto.frame_rate.num, dto.frame_rate.den).map_err(|e| {
        at.key("frame_rate")
            .bad(format!("unsupported frame rate: {e}"))
    })?;
    let sample_rate = SampleRate::new(dto.sample_rate).map_err(|e| {
        at.key("sample_rate")
            .bad(format!("unsupported sample rate: {e}"))
    })?;
    extras.put(Owner::Settings, "", dto.extra);
    extras.put(Owner::Settings, "canvas", dto.canvas.extra);
    extras.put(Owner::Settings, "frame_rate", dto.frame_rate.extra);
    Ok(ProjectSettings {
        width: dto.canvas.width,
        height: dto.canvas.height,
        frame_rate,
        sample_rate,
        match_first_clip: dto.match_first_clip,
    })
}

fn read_media(
    dto: MediaItemDto,
    at: &At,
    extras: &mut Extras,
) -> Result<Arc<MediaItem>, OpenError> {
    let media = MediaId::from_raw(id(dto.id, &at.key("id"))?);
    let owner = Owner::Media(media);
    let source = read_source(dto.source, &at.key("source"), owner, extras)?;
    let origin = match dto.origin {
        Some(origin) => read_origin(origin, &at.key("origin"), owner, extras)?,
        None => MediaOrigin::Imported,
    };
    extras.put(owner, "", dto.extra);
    extras.put(owner, "path", dto.path.extra);
    let info = read_info(dto.info, &at.key("info"), owner, extras)?;
    let fingerprint = match dto.fingerprint {
        Some(f) => {
            let head_tail = parse_hex64(&f.head_tail).ok_or_else(|| {
                at.key("fingerprint")
                    .key("head_tail")
                    .bad("expected 16 hex digits")
            })?;
            let duration = tick(f.duration, &at.key("fingerprint").key("duration"))?;
            extras.put(owner, "fingerprint", f.extra);
            Some(Fingerprint {
                size: f.size,
                head_tail,
                duration,
            })
        }
        None => None,
    };
    Ok(Arc::new(MediaItem {
        id: media,
        source,
        origin,
        path: StoredPath {
            absolute: dto.path.absolute,
            relative: dto.path.relative,
        },
        info,
        fingerprint,
        display_name: dto.display_name,
    }))
}

fn read_source(
    dto: Tagged<SourceDto>,
    at: &At,
    owner: Owner,
    extras: &mut Extras,
) -> Result<MediaSource, OpenError> {
    let variants: [Variant<SourceDto>; 2] = [
        ("file", |v, at| decode_at(v, at).map(SourceDto::File)),
        ("audio-of", |v, at| decode_at(v, at).map(SourceDto::AudioOf)),
    ];
    Ok(match decode_tagged(dto, at, "media source", &variants)? {
        SourceDto::File(file) => {
            extras.put(owner, "source", file.extra);
            MediaSource::File
        }
        SourceDto::AudioOf(audio_of) => {
            // The parent may have been removed since: the item has its own
            // copy of the path, so it stays valid (AU-004).
            let parent = MediaId::from_raw(id(audio_of.media, &at.key("media"))?);
            extras.put(owner, "source", audio_of.extra);
            MediaSource::AudioOf {
                source: parent,
                stream: audio_of.stream,
            }
        }
    })
}

/// Where a media item came from (format 9, PM-041).
fn read_origin(
    dto: Tagged<MediaOriginDto>,
    at: &At,
    owner: Owner,
    extras: &mut Extras,
) -> Result<MediaOrigin, OpenError> {
    let variants: [Variant<MediaOriginDto>; 2] = [
        ("imported", |v, at| {
            decode_at(v, at).map(MediaOriginDto::Imported)
        }),
        ("generated", |v, at| {
            decode_at(v, at).map(MediaOriginDto::Generated)
        }),
    ];
    Ok(match decode_tagged(dto, at, "media origin", &variants)? {
        MediaOriginDto::Imported(plain) => {
            extras.put(owner, "origin", plain.extra);
            MediaOrigin::Imported
        }
        MediaOriginDto::Generated(generated) => {
            extras.put(owner, "origin", generated.extra);
            MediaOrigin::Generated {
                kind: match generated.kind {
                    GeneratedMediaKindDto::Voiceover => GeneratedMediaKind::Voiceover,
                    GeneratedMediaKindDto::Tts => GeneratedMediaKind::Tts,
                },
            }
        }
    })
}

fn read_info(
    dto: MediaInfoDto,
    at: &At,
    owner: Owner,
    extras: &mut Extras,
) -> Result<MediaInfo, OpenError> {
    extras.put(owner, "info", dto.extra);
    let duration = tick(dto.duration, &at.key("duration"))?;
    let mut video = Vec::with_capacity(dto.video.len());
    for (i, v) in dto.video.into_iter().enumerate() {
        let stream_at = at.key("video").index(i);
        let rate = match &v.rate {
            Some(r) => Some(Rational::new(r.num, r.den).map_err(|e| stream_at.key("rate").bad(e))?),
            None => None,
        };
        let start = tick(v.start, &stream_at.key("start"))?;
        let stream_duration = tick(v.duration, &stream_at.key("duration"))?;
        if !v.extra.is_empty() {
            extras.put(owner, format!("info.video[{i}]"), v.extra);
        }
        if !v.sar.extra.is_empty() {
            extras.put(owner, format!("info.video[{i}].sar"), v.sar.extra);
        }
        if let Some(r) = v.rate
            && !r.extra.is_empty()
        {
            extras.put(owner, format!("info.video[{i}].rate"), r.extra);
        }
        video.push(VideoStreamInfo {
            index: v.index,
            codec: v.codec,
            width: v.width,
            height: v.height,
            sar: (v.sar.num, v.sar.den),
            rotation: match v.rotation {
                RotationDto::None => Rotation::None,
                RotationDto::Cw90 => Rotation::Cw90,
                RotationDto::Cw180 => Rotation::Cw180,
                RotationDto::Cw270 => Rotation::Cw270,
            },
            mirrored: v.mirrored,
            rate,
            variable_rate: v.variable_rate,
            has_alpha: v.has_alpha,
            bit_depth: v.bit_depth,
            start,
            duration: stream_duration,
        });
    }
    let mut audio = Vec::with_capacity(dto.audio.len());
    for (i, a) in dto.audio.into_iter().enumerate() {
        let stream_at = at.key("audio").index(i);
        let start = tick(a.start, &stream_at.key("start"))?;
        let stream_duration = tick(a.duration, &stream_at.key("duration"))?;
        if !a.extra.is_empty() {
            extras.put(owner, format!("info.audio[{i}]"), a.extra);
        }
        audio.push(AudioStreamInfo {
            index: a.index,
            codec: a.codec,
            sample_rate: a.sample_rate,
            channels: a.channels,
            start,
            duration: stream_duration,
            duration_exact: a.duration_exact,
            default: a.default,
            language: a.language,
            title: a.title,
            layout: a.layout,
        });
    }
    let still = dto.still.map(|s| {
        extras.put(owner, "info.still", s.extra);
        (s.width, s.height)
    });
    Ok(MediaInfo {
        container: dto.container,
        default_video: dto.default_video,
        duration,
        video,
        audio,
        still,
    })
}

fn read_sequence(
    dto: SequenceDto,
    at: &At,
    reader: &mut ClipReader<'_>,
) -> Result<SeqParts, OpenError> {
    let seq = SeqId::from_raw(id(dto.id, &at.key("id"))?);
    let extras = &mut *reader.extras;
    extras.put(Owner::Sequence(seq), "", dto.extra);
    extras.put(Owner::Sequence(seq), "flags", dto.flags.extra);
    let master = match dto.master {
        None => MasterAudio::default(),
        Some(m) => {
            extras.put(Owner::Sequence(seq), "master", m.extra);
            MasterAudio {
                gain: master_gain(m.gain, &at.key("master").key("gain"))?,
            }
        }
    };
    let read_lane = |tracks: Vec<TrackDto>, lane: &At, reader: &mut ClipReader<'_>| {
        tracks
            .into_iter()
            .enumerate()
            .map(|(i, t)| read_track(t, &lane.index(i), reader))
            .collect::<Result<Vec<_>, _>>()
    };
    let (markers, fixed) =
        read_markers(dto.markers, &at.key("markers"), false, &mut *reader.extras)?;
    reader.markers += usize::from(fixed);
    let visual = read_lane(dto.visual, &at.key("visual"), reader)?;
    let audio = read_lane(dto.audio, &at.key("audio"), reader)?;
    Ok(SeqParts {
        id: seq,
        flags: SequenceFlags {
            main_magnet: dto.flags.main_magnet,
            follow_main: dto.flags.follow_main,
            follow_kinds: dto
                .flags
                .follow_kinds
                .iter()
                .map(|k| follow_kind(*k))
                .collect(),
        },
        master,
        markers,
        visual,
        audio,
    })
}

fn follow_kind(dto: FollowKindDto) -> FollowKind {
    match dto {
        FollowKindDto::VideoOverlay => FollowKind::VideoOverlay,
        FollowKindDto::Text => FollowKind::Text,
        FollowKindDto::Sticker => FollowKind::Sticker,
        FollowKindDto::Effect => FollowKind::Effect,
        FollowKindDto::Adjustment => FollowKind::Adjustment,
        FollowKindDto::Music => FollowKind::Music,
        FollowKindDto::SoundEffect => FollowKind::SoundEffect,
        FollowKindDto::Speech => FollowKind::Speech,
        FollowKindDto::OtherAudio => FollowKind::OtherAudio,
    }
}

fn follow_kind_dto(kind: FollowKind) -> FollowKindDto {
    match kind {
        FollowKind::VideoOverlay => FollowKindDto::VideoOverlay,
        FollowKind::Text => FollowKindDto::Text,
        FollowKind::Sticker => FollowKindDto::Sticker,
        FollowKind::Effect => FollowKindDto::Effect,
        FollowKind::Adjustment => FollowKindDto::Adjustment,
        FollowKind::Music => FollowKindDto::Music,
        FollowKind::SoundEffect => FollowKindDto::SoundEffect,
        FollowKind::Speech => FollowKindDto::Speech,
        FollowKind::OtherAudio => FollowKindDto::OtherAudio,
    }
}

/// A marker list (format 7, TL-037), and whether it had to be fixed: put in
/// order, or a name or note shortened to its length limit, as an edit
/// would have (a hand-edited file). `on_clip` says whose list it is: beats
/// are clip markers only, and a beat on the timeline is refused.
fn read_markers(
    dtos: Vec<MarkerDto>,
    at: &At,
    on_clip: bool,
    extras: &mut Extras,
) -> Result<(Markers, bool), OpenError> {
    let mut fixed = false;
    let mut list = Vec::with_capacity(dtos.len());
    for (i, dto) in dtos.into_iter().enumerate() {
        let marker_at = at.index(i);
        let id = MarkerId::from_raw(id(dto.id, &marker_at.key("id"))?);
        let range_at = marker_at.key("at");
        let start = tick(dto.at.start, &range_at.key("start"))?;
        let duration = tick(dto.at.duration, &range_at.key("duration"))?;
        let range = MarkRange::new(start, duration).map_err(|e| range_at.bad(e))?;
        let origin = match dto.origin {
            MarkerOriginDto::User => MarkerOrigin::User,
            MarkerOriginDto::Beat => MarkerOrigin::Beat { auto: false },
            MarkerOriginDto::DetectedBeat => MarkerOrigin::Beat { auto: true },
            MarkerOriginDto::Scene => MarkerOrigin::Scene,
        };
        if !on_clip && matches!(origin, MarkerOrigin::Beat { .. }) {
            return Err(marker_at
                .key("origin")
                .bad("beat markers belong to clips, not to the timeline"));
        }
        let mut shortened = |text: String, limit: usize| {
            if text.chars().count() <= limit {
                return text;
            }
            fixed = true;
            text.chars().take(limit).collect()
        };
        let name = shortened(dto.name, MAX_MARKER_NAME_CHARS);
        let note = shortened(dto.note, MAX_MARKER_NOTE_CHARS);
        extras.put(Owner::Marker(id), "", dto.extra);
        extras.put(Owner::Marker(id), "at", dto.at.extra);
        list.push(Marker {
            id,
            at: range,
            name,
            note,
            color: match dto.color {
                PaletteColorDto::Blue => PaletteColor::Blue,
                PaletteColorDto::Teal => PaletteColor::Teal,
                PaletteColorDto::Green => PaletteColor::Green,
                PaletteColorDto::Yellow => PaletteColor::Yellow,
                PaletteColorDto::Orange => PaletteColor::Orange,
                PaletteColorDto::Red => PaletteColor::Red,
                PaletteColorDto::Pink => PaletteColor::Pink,
                PaletteColorDto::Purple => PaletteColor::Purple,
            },
            origin,
        });
    }
    // Two markers with one id are not "out of order": validation refuses
    // the duplicate id.
    fixed |= list
        .windows(2)
        .any(|w| (w[0].at.start(), w[0].id) > (w[1].at.start(), w[1].id));
    Ok((Markers::new(list), fixed))
}

fn read_track(
    dto: TrackDto,
    at: &At,
    reader: &mut ClipReader<'_>,
) -> Result<Arc<Track>, OpenError> {
    let track = TrackId::from_raw(id(dto.id, &at.key("id"))?);
    reader.extras.put(Owner::Track(track), "", dto.extra);
    reader
        .extras
        .put(Owner::Track(track), "header", dto.header.extra);
    let clips = dto
        .clips
        .into_iter()
        .enumerate()
        .map(|(i, c)| read_clip(c, &at.key("clips").index(i), reader))
        .collect::<Result<Vec<_>, _>>()?;
    let kind = match dto.kind {
        TrackKindDto::Video => TrackKind::Video,
        TrackKindDto::Text => TrackKind::Text,
        TrackKindDto::Sticker => TrackKind::Sticker,
        TrackKindDto::Effect => TrackKind::Effect,
        TrackKindDto::Adjustment => TrackKind::Adjustment,
        TrackKindDto::Audio => TrackKind::Audio,
    };
    let header = TrackHeader {
        name: dto.header.name,
        flags: TrackFlags {
            muted: dto.header.muted,
            hidden: dto.header.hidden,
            locked: dto.header.locked,
        },
    };
    let mix = match dto.mix {
        None => TrackAudio::default(),
        Some(m) => {
            let mix_at = at.key("mix");
            reader.extras.put(Owner::Track(track), "mix", m.extra);
            let mix = TrackAudio {
                gain: track_gain(m.gain, &mix_at.key("gain"))?,
                pan: pan(m.pan, &mix_at.key("pan"))?,
            };
            // AU-013: tracks without sound keep the defaults.
            if !kind.carries_sound() && mix != TrackAudio::default() {
                return Err(mix_at.bad(format!(
                    "a {kind:?} track carries no sound, so its volume and pan stay at the defaults"
                )));
            }
            mix
        }
    };
    let mut built = Track::from_parts(track, kind, header, clips);
    built.mix = mix;
    Ok(Arc::new(built))
}

fn read_clip(dto: ClipDto, at: &At, reader: &mut ClipReader<'_>) -> Result<Arc<Clip>, OpenError> {
    let clip = ClipId::from_raw(id(dto.id, &at.key("id"))?);
    let owner = Owner::Clip(clip);
    let extras = &mut *reader.extras;
    let mut clamped = false;
    let placed = range(&dto.range, &at.key("range"))?;
    let content_at = at.key("content");
    let content = match read_content(dto.content, &content_at)? {
        ContentDto::Media(media) => {
            let source_range = range(&media.source, &content_at.key("source"))?;
            let asset = MediaId::from_raw(id(media.asset, &content_at.key("asset"))?);
            let stream = match media.stream {
                Some(sel) => Some(read_stream(sel, &content_at.key("stream"), owner, extras)?),
                None => None,
            };
            extras.put(owner, "content", media.extra);
            extras.put(owner, "content.source", media.source.extra);
            ClipContent::Media {
                asset,
                kind: match media.kind {
                    MediaKindDto::Video => MediaKind::Video,
                    MediaKindDto::Audio => MediaKind::Audio,
                },
                source: source_range,
                stream,
            }
        }
        ContentDto::Image(image) => {
            let asset = MediaId::from_raw(id(image.asset, &content_at.key("asset"))?);
            extras.put(owner, "content", image.extra);
            ClipContent::Image { asset }
        }
        ContentDto::Text(text) => {
            let data = read_text(*text, &content_at, owner, extras, &mut clamped)?;
            let fixed = data.clamped();
            clamped |= fixed != data;
            ClipContent::Text(Arc::new(fixed))
        }
        ContentDto::Generated(generated) => {
            extras.put(owner, "content", generated.extra);
            ClipContent::Generated {
                kind: match generated.kind {
                    GeneratedKindDto::Sticker => GeneratedKind::Sticker,
                    GeneratedKindDto::Effect => GeneratedKind::Effect,
                    GeneratedKindDto::Adjustment => GeneratedKind::Adjustment,
                },
            }
        }
    };
    let group = match dto.group {
        Some(g) => Some(GroupId::from_raw(id(g, &at.key("group"))?)),
        None => None,
    };
    let link = match dto.link {
        Some(l) => Some(LinkId::from_raw(id(l, &at.key("link"))?)),
        None => None,
    };
    let (audio, mut sorted) = read_audio(dto.audio, &at.key("audio"), owner, extras)?;
    let extras = &mut *reader.extras;
    let transform = match dto.transform {
        None => Transform::IDENTITY,
        Some(t) => {
            let raw = read_transform(t, owner, extras, &mut clamped);
            let fixed = raw.clamped();
            clamped |= fixed != raw;
            fixed
        }
    };
    extras.put(owner, "", dto.extra);
    extras.put(owner, "range", dto.range.extra);
    let animations = match dto.animations {
        None => ClipAnimations::default(),
        Some(a) => motion::read_animations(
            a,
            &at.key("animations"),
            &content,
            owner,
            extras,
            &mut clamped,
        )?,
    };
    let (markers, markers_fixed) =
        read_markers(dto.markers, &at.key("markers"), true, &mut *extras)?;
    reader.markers += usize::from(markers_fixed);
    let extras = &mut *reader.extras;
    let attach = match dto.attach {
        None => None,
        Some(a) => {
            let attach_at = at.key("attach");
            let anchor = Attachment {
                owner: ClipId::from_raw(id(a.owner, &attach_at.key("owner"))?),
                owner_source_time: tick(a.owner_source_time, &attach_at.key("owner_source_time"))?,
            };
            extras.put(owner, "attach", a.extra);
            Some(anchor)
        }
    };
    let color = match dto.color {
        None => None,
        Some(c) => color::read_color(c, &at.key("color"), &content, owner, extras, &mut clamped)?
            .map(Arc::new),
    };
    let mut out = Clip {
        id: clip,
        range: placed,
        content,
        enabled: dto.enabled,
        name: dto.name,
        group,
        link,
        audio,
        transform,
        keys: ClipKeys::default(),
        animations,
        markers,
        attach,
        color,
    };
    if let Some(keys) = dto.keys {
        // Values are clamped against the clip's own content and transform.
        let (keys, keys_sorted) =
            motion::read_keys(keys, &at.key("keys"), &out, owner, extras, &mut clamped)?;
        out.keys = keys;
        sorted |= keys_sorted;
    }
    if clamped {
        reader.clamped += 1;
    }
    if sorted {
        reader.sorted += 1;
    }
    Ok(Arc::new(out))
}

/// A clip transform, as stored; the caller clamps it. Numbers beyond the
/// model's width are saturated, and `clamped` set.
fn read_transform(
    dto: TransformDto,
    owner: Owner,
    extras: &mut Extras,
    clamped: &mut bool,
) -> Transform {
    extras.put(owner, "transform", dto.extra);
    extras.put(owner, "transform.position", dto.position.extra);
    let mut i32_of = |v: i64| fit(v, i32::MIN, i32::MAX, clamped);
    let position = CanvasPos::new(i32_of(dto.position.x), i32_of(dto.position.y));
    let scale = Scale::from_basis_points(i32_of(dto.scale));
    let rotation = Angle::from_hundredths(i32_of(dto.rotation));
    Transform {
        position,
        scale,
        rotation,
        flip_h: dto.flip_h,
        flip_v: dto.flip_v,
        opacity: PerMille::new(fit(dto.opacity, 0, u16::MAX, clamped)),
    }
}

/// `v` as the model's integer type, saturated to `lo` or `hi` when it
/// doesn't fit; `clamped` is set then, so the open reports it like any
/// other value clamped into range (project-format.md, TR-001, TX-009).
fn fit<T: TryFrom<i64>>(v: i64, lo: T, hi: T, clamped: &mut bool) -> T {
    T::try_from(v).unwrap_or_else(|_| {
        *clamped = true;
        if v < 0 { lo } else { hi }
    })
}

/// `"#RRGGBBAA"`, hex digits in either case.
fn parse_color(text: &str) -> Option<Rgba8> {
    let hex = text.strip_prefix('#')?;
    if hex.len() != 8 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    Some(Rgba8::new(byte(0)?, byte(2)?, byte(4)?, byte(6)?))
}

/// How the file spells a color: `"#RRGGBBAA"`, uppercase.
fn color_text(c: Rgba8) -> String {
    format!("#{:02X}{:02X}{:02X}{:02X}", c.r, c.g, c.b, c.a)
}

/// The last export's settings (EX-021). Values the dialog can't offer are
/// refused with their path, like any other value; preflight checks the
/// combinations.
fn read_export(dto: ExportDto, at: &At, extras: &mut Extras) -> Result<ExportSettings, OpenError> {
    let owner = Owner::Export;
    extras.put(owner, "", dto.extra);
    extras.put(owner, "file", dto.file.extra);
    let video_at = at.key("video");
    let size_variants: [Variant<OutputSizeDto>; 2] = [
        ("canvas", |v, at| {
            decode_at(v, at).map(OutputSizeDto::Canvas)
        }),
        ("preset", |v, at| {
            decode_at(v, at).map(OutputSizeDto::Preset)
        }),
    ];
    let size_at = video_at.key("size");
    let size = match decode_tagged(dto.video.size, &size_at, "output size", &size_variants)? {
        OutputSizeDto::Canvas(plain) => {
            extras.put(owner, "video.size", plain.extra);
            OutputSize::Canvas
        }
        OutputSizeDto::Preset(preset) => {
            let side = ShortSide::from_px(preset.short_side).ok_or_else(|| {
                size_at.key("short_side").bad(format!(
                    "{} px is not a resolution preset (480, 720, 1080, 1440 or 2160)",
                    preset.short_side
                ))
            })?;
            extras.put(owner, "video.size", preset.extra);
            OutputSize::Preset(side)
        }
    };
    let rate_variants: [Variant<FrameRateChoiceDto>; 2] = [
        ("project", |v, at| {
            decode_at(v, at).map(FrameRateChoiceDto::Project)
        }),
        ("fixed", |v, at| {
            decode_at(v, at).map(FrameRateChoiceDto::Fixed)
        }),
    ];
    let rate_at = video_at.key("frame_rate");
    let frame_rate =
        match decode_tagged(dto.video.frame_rate, &rate_at, "frame rate", &rate_variants)? {
            FrameRateChoiceDto::Project(plain) => {
                extras.put(owner, "video.frame_rate", plain.extra);
                FrameRateChoice::Project
            }
            FrameRateChoiceDto::Fixed(fixed) => {
                let rate = FrameRate::new(fixed.rate.num, fixed.rate.den).map_err(|e| {
                    rate_at
                        .key("rate")
                        .bad(format!("unsupported frame rate: {e}"))
                })?;
                extras.put(owner, "video.frame_rate", fixed.extra);
                extras.put(owner, "video.frame_rate.rate", fixed.rate.extra);
                FrameRateChoice::Fixed(rate)
            }
        };
    let quality_variants: [Variant<QualityDto>; 1] =
        [("tier", |v, at| decode_at(v, at).map(QualityDto::Tier))];
    let quality = match decode_tagged(
        dto.video.quality,
        &video_at.key("quality"),
        "quality",
        &quality_variants,
    )? {
        QualityDto::Tier(tier) => {
            extras.put(owner, "video.quality", tier.extra);
            match tier.tier {
                QualityTierDto::Smaller => QualityTier::Smaller,
                QualityTierDto::Recommended => QualityTier::Recommended,
                QualityTierDto::Higher => QualityTier::Higher,
            }
        }
    };
    extras.put(owner, "video", dto.video.extra);
    let audio_at = at.key("audio");
    if !crate::export::AAC_KBPS_CHOICES.contains(&dto.audio.kbps) {
        return Err(audio_at.key("kbps").bad(format!(
            "{} kbps is not an AAC bitrate FreeCut offers ({:?})",
            dto.audio.kbps,
            crate::export::AAC_KBPS_CHOICES
        )));
    }
    let sample_rate = SampleRate::new(dto.audio.sample_rate).map_err(|e| {
        audio_at
            .key("sample_rate")
            .bad(format!("unsupported sample rate: {e}"))
    })?;
    extras.put(owner, "audio", dto.audio.extra);
    Ok(ExportSettings {
        file: FileTarget {
            name: dto.file.name,
            folder: dto.file.folder.map(PathBuf::from),
        },
        video: VideoSettings {
            size,
            frame_rate,
            codec: match dto.video.codec {
                VideoCodecDto::H264 => VideoCodec::H264,
            },
            container: match dto.video.container {
                ContainerDto::Mp4 => Container::Mp4,
            },
            quality,
        },
        audio: AudioExportSettings {
            include: dto.audio.include,
            kbps: dto.audio.kbps,
            sample_rate,
        },
    })
}

fn read_content(dto: Tagged<ContentDto>, at: &At) -> Result<ContentDto, OpenError> {
    let variants: [Variant<ContentDto>; 4] = [
        ("media", |v, at| decode_at(v, at).map(ContentDto::Media)),
        ("image", |v, at| decode_at(v, at).map(ContentDto::Image)),
        ("text", |v, at| decode_at(v, at).map(ContentDto::Text)),
        ("generated", |v, at| {
            decode_at(v, at).map(ContentDto::Generated)
        }),
    ];
    decode_tagged(dto, at, "clip content", &variants)
}

/// A media clip's stream choice (AU-038). An index the media doesn't have
/// is kept, as after a relink: the open reports it and the default stream
/// plays.
fn read_stream(
    dto: Tagged<StreamSelDto>,
    at: &At,
    owner: Owner,
    extras: &mut Extras,
) -> Result<StreamSel, OpenError> {
    let variants: [Variant<StreamSelDto>; 2] = [
        ("index", |v, at| decode_at(v, at).map(StreamSelDto::Index)),
        ("all-mixed", |v, at| {
            decode_at(v, at).map(StreamSelDto::AllMixed)
        }),
    ];
    Ok(
        match decode_tagged(dto, at, "audio stream choice", &variants)? {
            StreamSelDto::Index(index) => {
                extras.put(owner, "content.stream", index.extra);
                StreamSel::Index(index.index)
            }
            StreamSelDto::AllMixed(plain) => {
                extras.put(owner, "content.stream", plain.extra);
                StreamSel::AllMixed
            }
        },
    )
}

/// A clip's sound, and whether its volume keys had to be sorted. Whether
/// the sound fits the clip's content (AU-002) is checked with the rest of
/// the document by `Document::validate`.
fn read_audio(
    dto: Tagged<AudioDto>,
    at: &At,
    owner: Owner,
    extras: &mut Extras,
) -> Result<(EmbeddedAudio, bool), OpenError> {
    let variants: [Variant<AudioDto>; 3] = [
        ("none", |v, at| decode_at(v, at).map(AudioDto::None)),
        ("active", |v, at| decode_at(v, at).map(AudioDto::Active)),
        ("detached", |v, at| decode_at(v, at).map(AudioDto::Detached)),
    ];
    Ok(match decode_tagged(dto, at, "clip audio", &variants)? {
        AudioDto::None(none) => {
            extras.put(owner, "audio", none.extra);
            (EmbeddedAudio::None, false)
        }
        AudioDto::Active(active) => {
            let static_gain = gain(active.gain, &at.key("gain"))?;
            let (keys, sorted) = gain_keys(active.gain_keys, &at.key("gain_keys"), owner, extras)?;
            let audio = ClipAudio {
                // Keys are strictly increasing by now.
                gain: Animatable::from_parts(static_gain, keys)
                    .map_err(|e| at.key("gain_keys").bad(e))?,
                muted: active.muted,
                fade_in: fade(active.fade_in, &at.key("fade_in"))?,
                fade_out: fade(active.fade_out, &at.key("fade_out"))?,
                fade_in_curve: fade_curve(active.fade_in_curve),
                fade_out_curve: fade_curve(active.fade_out_curve),
            };
            extras.put(owner, "audio", active.extra);
            (EmbeddedAudio::Active(audio), sorted)
        }
        AudioDto::Detached(detached) => {
            let link = LinkId::from_raw(id(detached.link, &at.key("link"))?);
            extras.put(owner, "audio", detached.extra);
            (EmbeddedAudio::Detached(link), false)
        }
    })
}

/// Volume keyframes (AU-008), sorted by time, and whether they had to be.
/// Two keys at one time are refused: which one counts can't be told. Each
/// key's unknown keys are filed under its sorted position, where the writer
/// puts that key.
fn gain_keys(
    dtos: Vec<GainKeyDto>,
    at: &At,
    owner: Owner,
    extras: &mut Extras,
) -> Result<(Vec<Keyframe<GainDb>>, bool), OpenError> {
    let mut keys = Vec::with_capacity(dtos.len());
    for (i, k) in dtos.into_iter().enumerate() {
        let key_at = at.index(i);
        let key = Keyframe {
            at: tick(k.at, &key_at.key("at"))?,
            value: gain(k.gain, &key_at.key("gain"))?,
            interp: motion::interp(k.interp),
        };
        keys.push((i, key, k.extra));
    }
    let sorted = keys.windows(2).any(|w| w[0].1.at > w[1].1.at);
    // Stable, so of two keys at one time the later one is named below.
    keys.sort_by_key(|(_, key, _)| key.at);
    if let Some(w) = keys.windows(2).find(|w| w[0].1.at == w[1].1.at) {
        return Err(at.index(w[1].0).key("at").bad(format!(
            "two volume keyframes at {} ticks (`{}` and `{}`); keyframe times must differ",
            w[1].1.at.raw(),
            at.index(w[0].0).spelled(),
            at.index(w[1].0).spelled(),
        )));
    }
    Ok((
        keys.into_iter()
            .map(|(_, key, extra)| {
                if !extra.is_empty() {
                    extras.put(owner, gain_key_path(key.at), extra);
                }
                key
            })
            .collect(),
        sorted,
    ))
}

/// Where the unknown keys of the volume keyframe at `at` are filed: by its
/// time, which is unique on a clip, so they stay with that keyframe when an
/// edit adds or removes an earlier one, and go when it is deleted or moved
/// (PM-021).
fn gain_key_path(at: Tick) -> String {
    format!("audio.gain_keys@{}", at.raw())
}

fn fade_curve(dto: FadeCurveDto) -> FadeCurve {
    match dto {
        FadeCurveDto::Linear => FadeCurve::Linear,
        FadeCurveDto::Smooth => FadeCurve::Smooth,
        FadeCurveDto::Fast => FadeCurve::Fast,
        FadeCurveDto::Slow => FadeCurve::Slow,
    }
}

fn fade_curve_dto(curve: FadeCurve) -> FadeCurveDto {
    match curve {
        FadeCurve::Linear => FadeCurveDto::Linear,
        FadeCurve::Smooth => FadeCurveDto::Smooth,
        FadeCurve::Fast => FadeCurveDto::Fast,
        FadeCurve::Slow => FadeCurveDto::Slow,
    }
}

/// A clip gain: −∞, or within the gain range (AU-005). Out-of-range values
/// are refused rather than clamped.
fn gain(dto: GainDto, at: &At) -> Result<GainDb, OpenError> {
    let raw = match dto {
        GainDto::Silent => return Ok(GainDb::SILENT),
        GainDto::Hundredths(raw) => raw,
    };
    let (min, max) = (GainDb::MIN.hundredths(), GainDb::MAX.hundredths());
    match i32::try_from(raw) {
        Ok(hundredths) if (min..=max).contains(&hundredths) => {
            Ok(GainDb::from_hundredths(hundredths))
        }
        _ => Err(at.bad(format!(
            "a gain of {raw} hundredths of a decibel is outside {min}..={max} (silence is \
             \"{GAIN_SILENT}\")"
        ))),
    }
}

/// A master volume (AU-010): a gain no louder than +12 dB.
fn master_gain(dto: GainDto, at: &At) -> Result<GainDb, OpenError> {
    let gain = gain(dto, at)?;
    if gain > MasterAudio::MAX_GAIN {
        return Err(at.bad(format!(
            "a master gain of {} hundredths of a decibel is above {}",
            gain.hundredths(),
            MasterAudio::MAX_GAIN.hundredths()
        )));
    }
    Ok(gain)
}

/// A track volume (AU-013): a gain no louder than +12 dB.
fn track_gain(dto: GainDto, at: &At) -> Result<GainDb, OpenError> {
    let gain = gain(dto, at)?;
    if gain > TrackAudio::MAX_GAIN {
        return Err(at.bad(format!(
            "a track gain of {} hundredths of a decibel is above {}",
            gain.hundredths(),
            TrackAudio::MAX_GAIN.hundredths()
        )));
    }
    Ok(gain)
}

/// A pan (AU-013): −100 to +100 percent. Refused outside, like a gain.
fn pan(raw: i64, at: &At) -> Result<Pan, OpenError> {
    match i32::try_from(raw) {
        Ok(percent) if (-100..=100).contains(&percent) => Ok(Pan::new(percent)),
        _ => Err(at.bad(format!("a pan of {raw} is outside -100..=100"))),
    }
}

/// How the file spells `gain`.
fn gain_dto(gain: GainDb) -> GainDto {
    if gain.is_silent() {
        GainDto::Silent
    } else {
        GainDto::Hundredths(i64::from(gain.hundredths()))
    }
}

/// A fade length: ticks, never negative (AU-006).
fn fade(raw: i64, at: &At) -> Result<Tick, OpenError> {
    let length = tick(raw, at)?;
    if length.is_negative() {
        return Err(at.bad(format!("a fade length can't be negative ({raw} ticks)")));
    }
    Ok(length)
}

/// One variant of a tagged object: its `"type"` tag and the decoder of the
/// rest of the object.
type Variant<T> = (&'static str, fn(Value, &At) -> Result<T, OpenError>);

/// Decodes the tagged object at `at` by its `"type"` tag; `what` names the
/// object in errors. Each variant is decoded as a plain struct, so a schema
/// error inside it names its full JSON path (an internally tagged serde enum
/// would only name `at`).
fn decode_tagged<T>(
    dto: Tagged<T>,
    at: &At,
    what: &str,
    variants: &[Variant<T>],
) -> Result<T, OpenError> {
    let raw = match dto {
        Tagged::Decoded(variant) => return Ok(variant),
        Tagged::Unparsed(raw) => raw,
    };
    let mut object = match raw {
        Value::Object(object) => object,
        other => {
            return Err(at.schema(format!(
                "invalid type: {}, expected a {what} object",
                json_kind(&other)
            )));
        }
    };
    let tag_at = at.key("type");
    let tag = match object.remove("type") {
        Some(Value::String(tag)) => tag,
        Some(other) => {
            return Err(tag_at.schema(format!(
                "invalid type: {}, expected a {what} type string",
                json_kind(&other)
            )));
        }
        None => return Err(at.schema("missing field `type`")),
    };
    let mut rest = Value::Object(object);
    // Reading the object as a `Value` turned wrapped wide integers back into
    // numbers; wrap them again for the variant's unknown-key buffer.
    super::carry_wide_integers(&mut rest);
    match variants.iter().find(|(name, _)| *name == tag) {
        Some((_, decode)) => decode(rest, at),
        None => {
            let expected = variants
                .iter()
                .map(|(name, _)| format!("`{name}`"))
                .collect::<Vec<_>>()
                .join(", ");
            Err(tag_at.schema(format!(
                "unknown variant `{tag}`, expected one of {expected}"
            )))
        }
    }
}

/// How serde names a JSON value's type in error messages.
fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "sequence",
        Value::Object(_) => "map",
    }
}

/// Exactly 16 hex digits (either case) as a `u64`.
fn parse_hex64(text: &str) -> Option<u64> {
    if text.len() != 16 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(text, 16).ok()
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// Unknown keys to write back for one entity.
struct Give<'a> {
    extras: &'a Extras,
    owner: Owner,
    any: bool,
}

impl<'a> Give<'a> {
    fn new(extras: &'a Extras, owner: Owner) -> Give<'a> {
        Give {
            extras,
            owner,
            any: extras.has(owner),
        }
    }

    fn at(&self, at: &str) -> Fields {
        if self.any {
            self.extras.fields(self.owner, at)
        } else {
            Fields::new()
        }
    }

    /// Like [`Give::at`] for paths that have to be formatted.
    fn at_with(&self, at: impl FnOnce() -> String) -> Fields {
        if self.any {
            self.extras.fields(self.owner, &at())
        } else {
            Fields::new()
        }
    }

    /// `dto`, one variant of an object with variants, with the unknown keys
    /// at `at` attached as its `extra`. Those are filed by path whatever the
    /// variant ([`crate::extras`]), so after a detach or restore they may
    /// have been read from another variant: any key that `dto` writes
    /// itself (or the `type` tag) is dropped, or the object would hold it
    /// twice and the stale value would win on the next open.
    fn variant<T: serde::Serialize>(
        &self,
        at: &str,
        mut dto: T,
        extra: impl FnOnce(&mut T) -> &mut Fields,
    ) -> T {
        let mut fields = self.at(at);
        if !fields.is_empty() {
            let own = match serde_json::to_value(&dto) {
                Ok(Value::Object(own)) => own,
                _ => serde_json::Map::new(),
            };
            fields.retain(|key, _| key != "type" && !own.contains_key(key));
        }
        *extra(&mut dto) = fields;
        dto
    }
}

/// How the writer fills in each media item's relative path (PM-100).
#[derive(Debug, Clone, Copy)]
pub(crate) enum RelativePaths<'a> {
    /// As the document holds them.
    AsStored,
    /// Recomputed for a project file in the folder `to`, from paths that are
    /// relative to the folder `from` (Save As, Save a Copy).
    Rebase { from: &'a Path, to: &'a Path },
}

impl RelativePaths<'_> {
    fn of(&self, path: &StoredPath) -> Option<String> {
        match *self {
            RelativePaths::AsStored => path.relative.clone(),
            RelativePaths::Rebase { from, to } => rebase_relative(path, from, to),
        }
    }
}

/// A value the file format can't hold exactly: its JSON path and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NotWritable {
    pub path: String,
    pub message: String,
}

/// Checks that every number [`write`] would produce stays within the bounds
/// the reader accepts (module docs), so whatever is saved opens again.
pub(crate) fn check_writable(doc: &Document, ids_next: NonZeroU64) -> Result<(), NotWritable> {
    if ids_next.get() > MAX_IDS_NEXT {
        return Err(NotWritable {
            path: "ids_next".into(),
            message: format!("the id counter {ids_next} is above the largest {MAX_IDS_NEXT}"),
        });
    }
    let check = |raw: Tick, at: &dyn Fn() -> At| {
        if raw.raw().unsigned_abs() > MAX_TICK {
            Err(NotWritable {
                path: at().spelled(),
                message: format!(
                    "{} ticks is outside the supported range of ±{MAX_TICK}",
                    raw.raw()
                ),
            })
        } else {
            Ok(())
        }
    };
    let check_range = |r: TimeRange, at: &dyn Fn() -> At| {
        check(r.start(), &|| at().key("start"))?;
        check(r.duration(), &|| at().key("duration"))
    };
    let doc_at = At::root().key("document");
    for (i, item) in doc.library().items().enumerate() {
        let item_at = || doc_at.key("library").index(i);
        let info = &item.info;
        check(info.duration, &|| item_at().key("info").key("duration"))?;
        for (j, v) in info.video.iter().enumerate() {
            let at = || item_at().key("info").key("video").index(j);
            check(v.start, &|| at().key("start"))?;
            check(v.duration, &|| at().key("duration"))?;
        }
        for (j, a) in info.audio.iter().enumerate() {
            let at = || item_at().key("info").key("audio").index(j);
            check(a.start, &|| at().key("start"))?;
            check(a.duration, &|| at().key("duration"))?;
        }
        if let Some(f) = &item.fingerprint {
            check(f.duration, &|| item_at().key("fingerprint").key("duration"))?;
        }
        // `MediaOrigin` and `GeneratedMediaKind` are non-exhaustive: an
        // origin added in the timeline needs a format change before it
        // can be saved, rather than being written as another.
        if !matches!(
            item.origin,
            MediaOrigin::Imported
                | MediaOrigin::Generated {
                    kind: GeneratedMediaKind::Voiceover | GeneratedMediaKind::Tts
                }
        ) {
            return Err(NotWritable {
                path: item_at().key("origin").spelled(),
                message: format!(
                    "this format version can't hold the media origin {:?}",
                    item.origin
                ),
            });
        }
    }
    let check_markers = |markers: &Markers, at: &dyn Fn() -> At| {
        for (n, m) in markers.iter().enumerate() {
            check(m.at.start(), &|| at().index(n).key("at").key("start"))?;
            check(m.at.duration(), &|| at().index(n).key("at").key("duration"))?;
        }
        Ok(())
    };
    for (i, seq) in doc.sequences().enumerate() {
        let seq_at = || doc_at.key("sequences").index(i);
        check_markers(&seq.markers, &|| seq_at().key("markers"))?;
        let lanes = [
            ("visual", seq.visual_tracks()),
            ("audio", seq.audio_tracks()),
        ];
        for (lane, tracks) in lanes {
            for (j, track) in tracks.iter().enumerate() {
                for (k, clip) in track.clips().iter().enumerate() {
                    let clip_at = || seq_at().key(lane).index(j).key("clips").index(k);
                    check_range(clip.range, &|| clip_at().key("range"))?;
                    if let ClipContent::Media { source, .. } = &clip.content {
                        check_range(*source, &|| clip_at().key("content").key("source"))?;
                    }
                    // Fades are stored as set, not clamped to the clip.
                    if let Some(audio) = clip.audio.active() {
                        check(audio.fade_in, &|| clip_at().key("audio").key("fade_in"))?;
                        check(audio.fade_out, &|| clip_at().key("audio").key("fade_out"))?;
                        for (n, key) in audio.gain.keys().iter().enumerate() {
                            check(key.at, &|| {
                                clip_at().key("audio").key("gain_keys").index(n).key("at")
                            })?;
                        }
                    }
                    for (raw, below) in motion::ticks(clip) {
                        check(raw, &|| At(format!("{}.{below}", clip_at().0)))?;
                    }
                    check_markers(&clip.markers, &|| clip_at().key("markers"))?;
                    if let Some(attach) = clip.attach {
                        check(attach.owner_source_time, &|| {
                            clip_at().key("attach").key("owner_source_time")
                        })?;
                    }
                    // `TextOrigin` is non-exhaustive: an origin added in
                    // the timeline needs a format change before it can be
                    // saved, rather than being written as another.
                    if let ClipContent::Text(data) = &clip.content
                        && !matches!(data.origin, TextOrigin::Manual)
                    {
                        return Err(NotWritable {
                            path: clip_at().key("content").key("origin").spelled(),
                            message: format!(
                                "this format version can't hold the text origin {:?}",
                                data.origin
                            ),
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn write(
    doc: &Document,
    ids_next: NonZeroU64,
    export: Option<&ExportSettings>,
    ui: &UiState,
    extras: &Extras,
    relative: RelativePaths<'_>,
) -> EnvelopeDto {
    let settings = doc.settings();
    let set = Give::new(extras, Owner::Settings);
    EnvelopeDto {
        format: FORMAT_MAGIC.to_string(),
        format_version: CURRENT_FORMAT_VERSION,
        app: APP_ID.to_string(),
        ids_next,
        document: DocumentDto {
            root: nz(doc.root().get()),
            settings: SettingsDto {
                canvas: SizeDto {
                    width: settings.width,
                    height: settings.height,
                    extra: set.at("canvas"),
                },
                frame_rate: RateDto {
                    num: settings.frame_rate.num(),
                    den: settings.frame_rate.den(),
                    extra: set.at("frame_rate"),
                },
                sample_rate: settings.sample_rate.hz(),
                match_first_clip: settings.match_first_clip,
                extra: set.at(""),
            },
            library: doc
                .library()
                .items()
                .map(|m| write_media(m, extras, relative))
                .collect(),
            sequences: doc.sequences().map(|s| write_sequence(s, extras)).collect(),
            extra: extras.fields(Owner::Document, ""),
        },
        export: export.map(|e| write_export(e, extras)),
        ui: write_ui(doc, ui, extras),
        extra: extras.fields(Owner::Root, ""),
    }
}

/// The `ui` object, left out while it is empty and carries no unknown keys.
/// Only sound-carrying tracks of `doc` are written as soloed.
fn write_ui(doc: &Document, ui: &UiState, extras: &Extras) -> Option<UiDto> {
    let ui = ui.retained_for(doc);
    let extra = extras.fields(Owner::Ui, "");
    (!ui.is_empty() || !extra.is_empty()).then(|| UiDto {
        solo: ui.solo.iter().map(|t| nz(t.get())).collect(),
        extra,
    })
}

fn write_export(settings: &ExportSettings, extras: &Extras) -> ExportDto {
    let give = Give::new(extras, Owner::Export);
    let video = &settings.video;
    let size = match video.size {
        OutputSize::Canvas => OutputSizeDto::Canvas(give.variant(
            "video.size",
            PlainDto {
                extra: Fields::new(),
            },
            |d| &mut d.extra,
        )),
        OutputSize::Preset(side) => OutputSizeDto::Preset(give.variant(
            "video.size",
            PresetSizeDto {
                short_side: side.px(),
                extra: Fields::new(),
            },
            |d| &mut d.extra,
        )),
    };
    let frame_rate = match video.frame_rate {
        FrameRateChoice::Project => FrameRateChoiceDto::Project(give.variant(
            "video.frame_rate",
            PlainDto {
                extra: Fields::new(),
            },
            |d| &mut d.extra,
        )),
        FrameRateChoice::Fixed(rate) => FrameRateChoiceDto::Fixed(give.variant(
            "video.frame_rate",
            FixedRateDto {
                rate: RateDto {
                    num: rate.num(),
                    den: rate.den(),
                    extra: give.at("video.frame_rate.rate"),
                },
                extra: Fields::new(),
            },
            |d| &mut d.extra,
        )),
    };
    let quality = QualityDto::Tier(give.variant(
        "video.quality",
        TierDto {
            tier: match video.quality {
                QualityTier::Smaller => QualityTierDto::Smaller,
                QualityTier::Recommended => QualityTierDto::Recommended,
                QualityTier::Higher => QualityTierDto::Higher,
            },
            extra: Fields::new(),
        },
        |d| &mut d.extra,
    ));
    ExportDto {
        file: ExportFileDto {
            name: settings.file.name.clone(),
            // A folder whose path isn't Unicode can't be written into JSON;
            // it is forgotten, and the next export asks for a folder again.
            folder: settings
                .file
                .folder
                .as_deref()
                .and_then(Path::to_str)
                .map(String::from),
            extra: give.at("file"),
        },
        video: ExportVideoDto {
            size: Tagged::Decoded(size),
            frame_rate: Tagged::Decoded(frame_rate),
            codec: match video.codec {
                VideoCodec::H264 => VideoCodecDto::H264,
            },
            container: match video.container {
                Container::Mp4 => ContainerDto::Mp4,
            },
            quality: Tagged::Decoded(quality),
            extra: give.at("video"),
        },
        audio: ExportAudioDto {
            include: settings.audio.include,
            kbps: settings.audio.kbps,
            sample_rate: settings.audio.sample_rate.hz(),
            extra: give.at("audio"),
        },
        extra: give.at(""),
    }
}

/// Model ids are `NonZeroU64` already; this just re-wraps the raw value.
fn nz(raw: u64) -> NonZeroU64 {
    NonZeroU64::new(raw).unwrap_or(NonZeroU64::MIN)
}

fn write_range(r: TimeRange, extra: Fields) -> RangeDto {
    RangeDto {
        start: r.start().raw(),
        duration: r.duration().raw(),
        extra,
    }
}

fn write_media(item: &MediaItem, extras: &Extras, relative: RelativePaths<'_>) -> MediaItemDto {
    let give = Give::new(extras, Owner::Media(item.id));
    let info = &item.info;
    let source = match item.source {
        MediaSource::File => SourceDto::File(give.variant(
            "source",
            FileSourceDto {
                extra: Fields::new(),
            },
            |d| &mut d.extra,
        )),
        MediaSource::AudioOf { source, stream } => SourceDto::AudioOf(give.variant(
            "source",
            AudioOfSourceDto {
                media: nz(source.get()),
                stream,
                extra: Fields::new(),
            },
            |d| &mut d.extra,
        )),
    };
    let origin = match item.origin {
        MediaOrigin::Generated { kind } => Some(MediaOriginDto::Generated(give.variant(
            "origin",
            GeneratedOriginDto {
                kind: match kind {
                    GeneratedMediaKind::Tts => GeneratedMediaKindDto::Tts,
                    // Voiceover; `check_writable` refuses any newer kind
                    // before writing.
                    _ => GeneratedMediaKindDto::Voiceover,
                },
                extra: Fields::new(),
            },
            |d| &mut d.extra,
        ))),
        // Imported (`check_writable` refuses any newer origin): left out,
        // unless it carries unknown keys.
        _ => {
            let extra = give.at("origin");
            (!extra.is_empty()).then(|| {
                MediaOriginDto::Imported(give.variant(
                    "origin",
                    PlainDto {
                        extra: Fields::new(),
                    },
                    |d| &mut d.extra,
                ))
            })
        }
    };
    MediaItemDto {
        id: nz(item.id.get()),
        source: Tagged::Decoded(source),
        origin: origin.map(Tagged::Decoded),
        path: StoredPathDto {
            absolute: item.path.absolute.clone(),
            relative: relative.of(&item.path),
            extra: give.at("path"),
        },
        info: MediaInfoDto {
            container: info.container.clone(),
            default_video: info.default_video,
            duration: info.duration.raw(),
            video: info
                .video
                .iter()
                .enumerate()
                .map(|(i, v)| VideoStreamDto {
                    index: v.index,
                    codec: v.codec.clone(),
                    width: v.width,
                    height: v.height,
                    sar: RateDto {
                        num: v.sar.0,
                        den: v.sar.1,
                        extra: give.at_with(|| format!("info.video[{i}].sar")),
                    },
                    rotation: match v.rotation {
                        Rotation::None => RotationDto::None,
                        Rotation::Cw90 => RotationDto::Cw90,
                        Rotation::Cw180 => RotationDto::Cw180,
                        Rotation::Cw270 => RotationDto::Cw270,
                    },
                    mirrored: v.mirrored,
                    rate: v.rate.map(|r| RationalDto {
                        num: r.num(),
                        den: r.den(),
                        extra: give.at_with(|| format!("info.video[{i}].rate")),
                    }),
                    variable_rate: v.variable_rate,
                    has_alpha: v.has_alpha,
                    bit_depth: v.bit_depth,
                    start: v.start.raw(),
                    duration: v.duration.raw(),
                    extra: give.at_with(|| format!("info.video[{i}]")),
                })
                .collect(),
            audio: info
                .audio
                .iter()
                .enumerate()
                .map(|(i, a)| AudioStreamDto {
                    index: a.index,
                    codec: a.codec.clone(),
                    sample_rate: a.sample_rate,
                    channels: a.channels,
                    start: a.start.raw(),
                    duration: a.duration.raw(),
                    duration_exact: a.duration_exact,
                    default: a.default,
                    language: a.language.clone(),
                    title: a.title.clone(),
                    layout: a.layout.clone(),
                    extra: give.at_with(|| format!("info.audio[{i}]")),
                })
                .collect(),
            still: info.still.map(|(width, height)| SizeDto {
                width,
                height,
                extra: give.at("info.still"),
            }),
            extra: give.at("info"),
        },
        fingerprint: item.fingerprint.map(|f| FingerprintDto {
            size: f.size,
            head_tail: format!("{:016x}", f.head_tail),
            duration: f.duration.raw(),
            extra: give.at("fingerprint"),
        }),
        display_name: item.display_name.clone(),
        extra: give.at(""),
    }
}

fn write_sequence(seq: &Sequence, extras: &Extras) -> SequenceDto {
    let give = Give::new(extras, Owner::Sequence(seq.id));
    let lane = |tracks: &[Arc<Track>]| tracks.iter().map(|t| write_track(t, extras)).collect();
    SequenceDto {
        id: nz(seq.id.get()),
        flags: SequenceFlagsDto {
            main_magnet: seq.flags.main_magnet,
            follow_main: seq.flags.follow_main,
            follow_kinds: seq.flags.follow_kinds.iter().map(follow_kind_dto).collect(),
            extra: give.at("flags"),
        },
        master: {
            let extra = give.at("master");
            (seq.master != MasterAudio::default() || !extra.is_empty()).then(|| MasterAudioDto {
                gain: gain_dto(seq.master.gain),
                extra,
            })
        },
        markers: write_markers(&seq.markers, extras),
        visual: lane(seq.visual_tracks()),
        audio: lane(seq.audio_tracks()),
        extra: give.at(""),
    }
}

fn write_track(track: &Track, extras: &Extras) -> TrackDto {
    let give = Give::new(extras, Owner::Track(track.id));
    TrackDto {
        id: nz(track.id.get()),
        kind: match track.kind {
            TrackKind::Video => TrackKindDto::Video,
            TrackKind::Text => TrackKindDto::Text,
            TrackKind::Sticker => TrackKindDto::Sticker,
            TrackKind::Effect => TrackKindDto::Effect,
            TrackKind::Adjustment => TrackKindDto::Adjustment,
            TrackKind::Audio => TrackKindDto::Audio,
        },
        header: TrackHeaderDto {
            name: track.header.name.clone(),
            muted: track.header.flags.muted,
            hidden: track.header.flags.hidden,
            locked: track.header.flags.locked,
            extra: give.at("header"),
        },
        mix: {
            let extra = give.at("mix");
            (track.mix != TrackAudio::default() || !extra.is_empty()).then(|| TrackMixDto {
                gain: gain_dto(track.mix.gain),
                pan: i64::from(track.mix.pan.percent()),
                extra,
            })
        },
        clips: track
            .clips()
            .iter()
            .map(|c| write_clip(c, extras))
            .collect(),
        extra: give.at(""),
    }
}

fn write_clip(clip: &Clip, extras: &Extras) -> ClipDto {
    let give = Give::new(extras, Owner::Clip(clip.id));
    let content = match &clip.content {
        ClipContent::Media {
            asset,
            kind,
            source,
            stream,
        } => ContentDto::Media(give.variant(
            "content",
            MediaContentDto {
                asset: nz(asset.get()),
                kind: match kind {
                    MediaKind::Video => MediaKindDto::Video,
                    MediaKind::Audio => MediaKindDto::Audio,
                },
                source: write_range(*source, give.at("content.source")),
                stream: stream.map(|sel| Tagged::Decoded(write_stream(sel, &give))),
                extra: Fields::new(),
            },
            |d| &mut d.extra,
        )),
        ClipContent::Image { asset } => ContentDto::Image(give.variant(
            "content",
            ImageContentDto {
                asset: nz(asset.get()),
                extra: Fields::new(),
            },
            |d| &mut d.extra,
        )),
        ClipContent::Text(data) => ContentDto::Text(Box::new(give.variant(
            "content",
            write_text(data, &give),
            |d| &mut d.extra,
        ))),
        ClipContent::Generated { kind } => ContentDto::Generated(give.variant(
            "content",
            GeneratedContentDto {
                kind: match kind {
                    GeneratedKind::Sticker => GeneratedKindDto::Sticker,
                    GeneratedKind::Effect => GeneratedKindDto::Effect,
                    GeneratedKind::Adjustment => GeneratedKindDto::Adjustment,
                },
                extra: Fields::new(),
            },
            |d| &mut d.extra,
        )),
    };
    let audio = match &clip.audio {
        EmbeddedAudio::None => AudioDto::None(give.variant(
            "audio",
            NoAudioDto {
                extra: Fields::new(),
            },
            |d| &mut d.extra,
        )),
        EmbeddedAudio::Active(audio) => AudioDto::Active(
            give.variant(
                "audio",
                ActiveAudioDto {
                    gain: gain_dto(*audio.gain.static_value()),
                    gain_keys: audio
                        .gain
                        .keys()
                        .iter()
                        .map(|key| GainKeyDto {
                            at: key.at.raw(),
                            gain: gain_dto(key.value),
                            interp: motion::interp_dto(key.interp),
                            extra: give.at_with(|| gain_key_path(key.at)),
                        })
                        .collect(),
                    muted: audio.muted,
                    fade_in: audio.fade_in.raw(),
                    fade_in_curve: fade_curve_dto(audio.fade_in_curve),
                    fade_out: audio.fade_out.raw(),
                    fade_out_curve: fade_curve_dto(audio.fade_out_curve),
                    extra: Fields::new(),
                },
                |d| &mut d.extra,
            ),
        ),
        EmbeddedAudio::Detached(link) => AudioDto::Detached(give.variant(
            "audio",
            DetachedAudioDto {
                link: nz(link.get()),
                extra: Fields::new(),
            },
            |d| &mut d.extra,
        )),
    };
    ClipDto {
        id: nz(clip.id.get()),
        range: write_range(clip.range, give.at("range")),
        content: Tagged::Decoded(content),
        enabled: clip.enabled,
        name: clip.name.clone(),
        group: clip.group.map(|g| nz(g.get())),
        link: clip.link.map(|l| nz(l.get())),
        audio: Tagged::Decoded(audio),
        transform: write_transform(&clip.transform, &give),
        keys: motion::write_keys(&clip.keys, &give),
        animations: motion::write_animations(&clip.animations, &give),
        markers: write_markers(&clip.markers, extras),
        attach: clip.attach.map(|a| AttachmentDto {
            owner: nz(a.owner.get()),
            owner_source_time: a.owner_source_time.raw(),
            extra: give.at("attach"),
        }),
        color: color::write_color(clip.color.as_deref(), &give),
        extra: give.at(""),
    }
}

/// A marker list (format 7), already in `(start, id)` order.
fn write_markers(markers: &Markers, extras: &Extras) -> Vec<MarkerDto> {
    markers
        .iter()
        .map(|m| {
            let give = Give::new(extras, Owner::Marker(m.id));
            MarkerDto {
                id: nz(m.id.get()),
                at: RangeDto {
                    start: m.at.start().raw(),
                    duration: m.at.duration().raw(),
                    extra: give.at("at"),
                },
                name: m.name.clone(),
                note: m.note.clone(),
                color: match m.color {
                    PaletteColor::Blue => PaletteColorDto::Blue,
                    PaletteColor::Teal => PaletteColorDto::Teal,
                    PaletteColor::Green => PaletteColorDto::Green,
                    PaletteColor::Yellow => PaletteColorDto::Yellow,
                    PaletteColor::Orange => PaletteColorDto::Orange,
                    PaletteColor::Red => PaletteColorDto::Red,
                    PaletteColor::Pink => PaletteColorDto::Pink,
                    PaletteColor::Purple => PaletteColorDto::Purple,
                },
                origin: match m.origin {
                    MarkerOrigin::User => MarkerOriginDto::User,
                    MarkerOrigin::Beat { auto: false } => MarkerOriginDto::Beat,
                    MarkerOrigin::Beat { auto: true } => MarkerOriginDto::DetectedBeat,
                    MarkerOrigin::Scene => MarkerOriginDto::Scene,
                },
                extra: give.at(""),
            }
        })
        .collect()
}

fn write_stream(sel: StreamSel, give: &Give<'_>) -> StreamSelDto {
    match sel {
        StreamSel::Index(index) => StreamSelDto::Index(give.variant(
            "content.stream",
            StreamIndexDto {
                index,
                extra: Fields::new(),
            },
            |d| &mut d.extra,
        )),
        StreamSel::AllMixed => StreamSelDto::AllMixed(give.variant(
            "content.stream",
            PlainDto {
                extra: Fields::new(),
            },
            |d| &mut d.extra,
        )),
    }
}

/// The transform object, left out when it is the identity and carries no
/// unknown keys.
fn write_transform(t: &Transform, give: &Give<'_>) -> Option<TransformDto> {
    let extra = give.at("transform");
    let position_extra = give.at("transform.position");
    if *t == Transform::IDENTITY && extra.is_empty() && position_extra.is_empty() {
        return None;
    }
    Some(TransformDto {
        position: PositionDto {
            x: i64::from(t.position.x),
            y: i64::from(t.position.y),
            extra: position_extra,
        },
        scale: i64::from(t.scale.basis_points()),
        rotation: i64::from(t.rotation.hundredths()),
        flip_h: t.flip_h,
        flip_v: t.flip_v,
        opacity: i64::from(t.opacity.get()),
        extra,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_paths_use_serde_path_to_error_notation() {
        let at = At::root()
            .key("document")
            .key("sequences")
            .index(0)
            .key("visual")
            .index(2)
            .key("id");
        assert_eq!(at.0, "document.sequences[0].visual[2].id");
        match At::root().bad("x") {
            OpenError::BadValue { path, .. } => assert_eq!(path, "."),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn hex_hashes_need_exactly_sixteen_digits() {
        assert_eq!(parse_hex64("00000000000000ff"), Some(255));
        assert_eq!(parse_hex64("FFFFFFFFFFFFFFFF"), Some(u64::MAX));
        assert_eq!(parse_hex64("ff"), None);
        assert_eq!(parse_hex64("+00000000000000f"), None);
        assert_eq!(parse_hex64("00000000000000fg"), None);
    }
}
